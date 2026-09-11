//! Abstract input method editors.

use std::{
    any::Any,
    cmp::{max, min},
    error::Error,
    fmt::{Debug, Display},
    fs::{self, File},
    hash::{DefaultHasher, Hash, Hasher},
    io::{BufReader, BufWriter},
    mem,
    path::PathBuf,
};

use log::{debug, error, info, warn};
use scoped_error::{ErrorExt, expect_error, impl_context_error};

pub use self::{abbrev::AbbrevTable, selection::symbol::SymbolSelector};
use self::{
    composition_editor::CompositionEditor,
    selection::{phrase::PhraseSelector, symbol::SpecialSymbolSelector},
    zhuyin_layout::{KeyBehavior, Standard, SyllableEditor},
};
use crate::{
    conversion::{
        ChewingEngine, ConversionEngine, Decoder, Interval, LatticeBuilder, Outcome, Selection,
        SimpleEngine, Symbol, full_width_symbol_input, special_symbol_input,
    },
    dictionary::{CompositeDict, LookupStrategy, StringTable},
    exn::{Exn, ResultExt},
    input::{KeyState, KeyboardEvent, keysym::*},
    lm::{LoadMode, StaticDict, StaticLm},
    path::SearchPath,
    user::{HistoryDict, UserDict, migrate_v3_to_v4, should_migrate_v3},
    zhuyin::{Syllable, SyllableVec},
};

mod abbrev;
mod composition_editor;
mod selection;
pub mod zhuyin_layout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageMode {
    Chinese,
    English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterForm {
    Halfwidth,
    Fullwidth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserPhraseAddDirection {
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionEngineKind {
    SimpleEngine,
    ChewingEngine,
    FuzzyChewingEngine,
}

#[derive(Debug, Clone, Copy)]
pub struct EditorOptions {
    pub easy_symbol_input: bool,
    pub esc_clear_all_buffer: bool,
    pub space_is_select_key: bool,
    pub auto_shift_cursor: bool,
    pub phrase_choice_rearward: bool,
    pub disable_auto_learn_phrase: bool,
    pub auto_commit_threshold: usize,
    pub candidates_per_page: usize,
    pub language_mode: LanguageMode,
    pub character_form: CharacterForm,
    pub user_phrase_add_dir: UserPhraseAddDirection,
    pub conversion_engine: ConversionEngineKind,
    pub enable_fullwidth_toggle_key: bool,
    pub sort_candidates_by_frequency: bool,
    pub auto_snapshot_selections: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            easy_symbol_input: false,
            esc_clear_all_buffer: false,
            space_is_select_key: false,
            auto_shift_cursor: false,
            phrase_choice_rearward: false,
            disable_auto_learn_phrase: false,
            auto_commit_threshold: 39,
            candidates_per_page: 10,
            language_mode: LanguageMode::Chinese,
            character_form: CharacterForm::Halfwidth,
            user_phrase_add_dir: UserPhraseAddDirection::Forward,
            conversion_engine: ConversionEngineKind::ChewingEngine,
            enable_fullwidth_toggle_key: true,
            sort_candidates_by_frequency: false,
            auto_snapshot_selections: false,
        }
    }
}

/// An editor can react to KeyEvents and change its state.
pub trait BasicEditor {
    /// Handles a KeyEvent
    fn process_keyevent(&mut self, evt: KeyboardEvent) -> EditorKeyBehavior;
}

/// The internal state of the editor.
trait State: Any + Debug {
    /// Transits the state to next state with the key event.
    fn next(&mut self, shared: &mut SharedState, ev: KeyboardEvent) -> Transition;

    fn spin_ignore(&self) -> Transition {
        Transition::Spin(EditorKeyBehavior::Ignore)
    }
    fn spin_absorb(&self) -> Transition {
        Transition::Spin(EditorKeyBehavior::Absorb)
    }
    fn spin_bell(&self) -> Transition {
        Transition::Spin(EditorKeyBehavior::Bell)
    }
}

#[derive(Debug)]
enum Transition {
    ToState(Box<dyn State>),
    Spin(EditorKeyBehavior),
}

/// Indicates the state change of the editor.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum EditorKeyBehavior {
    /// The key has no effect so it was ignored.
    Ignore,
    /// The key caused a conversion update.
    Commit,
    /// The key is an error.
    Bell,
    /// The key changed the editing state so was absorbed.
    Absorb,
}

#[derive(Debug)]
pub struct Editor {
    shared: SharedState,
    state: Box<dyn State>,
}

#[derive(Debug)]
pub(crate) struct SharedState {
    // static_words_path: PathBuf,
    // static_dict_path: PathBuf,
    user_datadir: Option<PathBuf>,
    // static_words: StringTable,
    // static_dict: StaticDict,
    com: CompositionEditor,
    syl: Box<dyn SyllableEditor>,
    conv: Box<dyn ConversionEngine>,
    string_table: StringTable,
    dict: CompositeDict,
    user_dict: UserDict,
    hist_dict: HistoryDict,
    decoder: Decoder,
    abbr: AbbrevTable,
    sym_sel: SymbolSelector,
    options: EditorOptions,
    last_key_behavior: EditorKeyBehavior,

    dirty_level: u16,
    nth_conversion: usize,
    commit_buffer: String,
    notice_buffer: String,
}

impl Editor {
    pub fn chewing(
        search_path: Option<String>,
        userpath: Option<String>,
    ) -> Result<Editor, NewEditorError> {
        expect_error("Failed to initialize new chewing Editor", || {
            let sp = match (search_path, userpath) {
                (Some(s), Some(u)) => SearchPath::from_system_path_and_user_path(&s, &u),
                (Some(s), None) => SearchPath::from_system_path_and_env(&s),
                (None, Some(u)) => SearchPath::from_user_path_and_env(&u),
                (None, None) => SearchPath::from_env(),
            };

            let static_dict_path = sp
                .find_file("static_dict.bin")
                .ok_or("Failed to find static_dict.bin file")?;
            let rare_dict_path = sp
                .find_file("rare_dict.bin")
                .ok_or("Failed to find rare_dict.bin file")?;
            let static_words_path = sp
                .find_file("static_words.bin")
                .ok_or("Failed to find static_words.bin file")?;
            let static_lm_path = sp
                .find_file("static_lm.bin")
                .ok_or("Failed to find static_lm.bin file")?;

            let static_dict = StaticDict::open(&static_dict_path)?;
            let rare_dict = StaticDict::open(&rare_dict_path)?;
            let string_table = StringTable::open_bin(&static_words_path)?;

            let lm = StaticLm::from_reader(
                BufReader::new(File::open(&static_lm_path)?),
                // Lazy mode is too slow for now
                LoadMode::Lazy,
            )?;

            if let Some(up) = sp.user_datadir() {
                if should_migrate_v3(up) {
                    migrate_v3_to_v4(up)?;
                }
            }

            let user_datadir = sp.user_versioned_path();

            if let Some(vp) = &user_datadir {
                fs::create_dir_all(vp)?;
            }

            let mut user_dict_path = sp.find_user_file("user_dict.csv");
            if user_dict_path.is_none() {
                if let Some(path) = sp.user_file_path("user_dict.csv") {
                    if let Err(err) = UserDict::init(path) {
                        error!("{}", err.report());
                    }
                }
                // try again
                user_dict_path = sp.find_user_file("user_dict.csv");
            }
            let user_dict = match user_dict_path {
                Some(path) => match UserDict::open(&path, string_table.clone()) {
                    Ok(dict) => dict,
                    Err(err) => {
                        error!("{}", err.report());
                        UserDict::new(string_table.clone())
                    }
                },
                None => UserDict::new(string_table.clone()),
            };

            let mut history_dict_path = sp.find_user_file("history_dict.bin");
            if history_dict_path.is_none() {
                if let Some(path) = sp.user_file_path("history_dict.bin") {
                    if let Err(err) = HistoryDict::init(path) {
                        error!("{}", err.report());
                    }
                }
                // try again
                history_dict_path = sp.find_user_file("history_dict.bin");
            }
            let hist_dict = match history_dict_path {
                Some(path) => match HistoryDict::open(&path, string_table.clone()) {
                    Ok(dict) => dict,
                    Err(err) => {
                        error!("{}", err.report());
                        HistoryDict::new(string_table.clone())
                    }
                },
                None => HistoryDict::new(string_table.clone()),
            };

            let composite_dict =
                CompositeDict::new(static_dict, rare_dict, hist_dict.clone(), user_dict.clone());

            let word_lattice_builder = LatticeBuilder {
                dict: composite_dict.clone(),
                lookup_strategy: LookupStrategy::Standard,
            };

            let decoder = Decoder {
                lm,
                alpha: Decoder::ALPHA,
            };

            let conversion_engine = Box::new(ChewingEngine {
                word_lattice_builder,
                decoder: decoder.clone(),
                string_table: string_table.clone(),
            });

            let abbrev = match sp.find_file("swkb.dat") {
                Some(swkb_dat) => AbbrevTable::open(swkb_dat)?,
                None => AbbrevTable::new(),
            };
            let sym_sel = match sp.find_file("symbols.dat") {
                Some(symbols_dat) => SymbolSelector::open(symbols_dat)?,
                None => SymbolSelector::new(b"".as_slice())?,
            };

            let editor = Editor::new(
                user_datadir,
                conversion_engine,
                string_table,
                composite_dict,
                user_dict,
                hist_dict,
                decoder,
                abbrev,
                sym_sel,
            );
            Ok(editor)
        })
    }

    pub fn new(
        user_datadir: Option<PathBuf>,
        conv: Box<dyn ConversionEngine>,
        string_table: StringTable,
        dict: CompositeDict,
        user_dict: UserDict,
        hist_dict: HistoryDict,
        decoder: Decoder,
        abbr: AbbrevTable,
        sym_sel: SymbolSelector,
    ) -> Editor {
        Editor {
            shared: SharedState {
                user_datadir,
                com: CompositionEditor::default(),
                syl: Box::new(Standard::new()),
                conv,
                string_table,
                dict,
                user_dict,
                hist_dict,
                decoder,
                abbr,
                sym_sel,
                options: EditorOptions::default(),
                last_key_behavior: EditorKeyBehavior::Absorb,
                dirty_level: 0,
                nth_conversion: 0,
                commit_buffer: String::new(),
                notice_buffer: String::new(),
            },
            state: Box::new(Entering),
        }
    }

    pub fn fallback() -> Editor {
        EditorBuilder::new().build()
    }

    pub fn set_syllable_editor(&mut self, syl: Box<dyn SyllableEditor>) {
        self.shared.syl = syl;
        info!("Set syllable editor: {}", self.shared.syl);
    }
    pub fn set_conversion_engine(&mut self, engine: Box<dyn ConversionEngine>) {
        self.shared.conv = engine;
        info!("Set conversion engine: {:?}", self.shared.conv);
    }
    pub fn clear(&mut self) {
        self.state = Box::new(Entering);
        self.shared.clear();
    }
    pub fn ack(&mut self) {
        self.shared.commit_buffer.clear();
    }
    pub fn clear_composition_editor(&mut self) {
        self.shared.com.clear();
    }
    pub fn clear_syllable_editor(&mut self) {
        self.shared.syl.clear();
    }
    pub fn cursor(&self) -> usize {
        self.shared.cursor()
    }

    // TODO: deprecate other direct set methods
    pub fn editor_options(&self) -> EditorOptions {
        self.shared.options
    }
    pub fn set_editor_options<F>(&mut self, update_op: F)
    where
        F: FnOnce(&mut EditorOptions),
    {
        let old = self.shared.options;
        update_op(&mut self.shared.options);
        if self.shared.options.language_mode != old.language_mode {
            self.cancel_entering_syllable();
        }
        if self.shared.options.conversion_engine != old.conversion_engine {
            self.shared.conv = match self.shared.options.conversion_engine {
                ConversionEngineKind::SimpleEngine => Box::new(SimpleEngine {
                    string_table: self.shared.string_table.clone(),
                    dict: self.shared.dict.clone(),
                }),
                ConversionEngineKind::ChewingEngine => Box::new(ChewingEngine {
                    word_lattice_builder: LatticeBuilder {
                        dict: self.shared.dict.clone(),
                        lookup_strategy: LookupStrategy::Standard,
                    },
                    decoder: self.shared.decoder.clone(),
                    string_table: self.shared.string_table.clone(),
                }),
                ConversionEngineKind::FuzzyChewingEngine => Box::new(ChewingEngine {
                    word_lattice_builder: LatticeBuilder {
                        dict: self.shared.dict.clone(),
                        lookup_strategy: LookupStrategy::FuzzyPartialPrefix,
                    },
                    decoder: self.shared.decoder.clone(),
                    string_table: self.shared.string_table.clone(),
                }),
            }
        }
    }
    pub fn entering_syllable(&self) -> bool {
        !self.shared.syl.is_empty()
    }
    pub fn syllable_buffer(&self) -> Syllable {
        self.shared.syl.read()
    }
    pub fn syllable_buffer_display(&self) -> String {
        self.shared
            .syl
            .key_seq()
            .unwrap_or_else(|| self.shared.syl.read().to_string())
    }
    pub fn symbols(&self) -> &[Symbol] {
        self.shared.com.symbols()
    }
    pub fn user_dict(&self) -> &UserDict {
        &self.shared.user_dict
    }
    pub fn learn_phrase(
        &mut self,
        syllables: &[Syllable],
        phrase: &str,
    ) -> Result<(), EditorError> {
        self.shared
            .learn_phrase(syllables, phrase)
            .or_raise(|| EditorError::new(EditorErrorKind::InvalidState))
    }
    pub fn unlearn_phrase(
        &mut self,
        syllables: &[Syllable],
        phrase: &str,
    ) -> Result<(), EditorError> {
        self.shared
            .unlearn_phrase(syllables, phrase)
            .or_raise(|| EditorError::new(EditorErrorKind::InvalidState))
    }
    /// All candidates after current page
    pub fn paginated_candidates(&self) -> Result<Vec<String>, EditorError> {
        let any = self.state.as_ref() as &dyn Any;
        if let Some(selecting) = any.downcast_ref::<Selecting>() {
            Ok(selecting
                .candidates(&self.shared)
                .into_iter()
                .skip(selecting.page_no * self.shared.options.candidates_per_page)
                .collect())
        } else {
            Err(EditorError::new(EditorErrorKind::InvalidState))
        }
    }
    pub fn all_candidates(&self) -> Result<Vec<String>, EditorError> {
        let any = self.state.as_ref() as &dyn Any;
        if let Some(selecting) = any.downcast_ref::<Selecting>() {
            Ok(selecting.candidates(&self.shared))
        } else {
            Err(EditorError::new(EditorErrorKind::InvalidState))
        }
    }
    pub fn current_page_no(&self) -> Result<usize, EditorError> {
        let any = self.state.as_ref() as &dyn Any;
        if let Some(selecting) = any.downcast_ref::<Selecting>() {
            Ok(selecting.page_no)
        } else {
            Err(EditorError::new(EditorErrorKind::InvalidState))
        }
    }
    pub fn total_page(&self) -> Result<usize, EditorError> {
        let any = self.state.as_ref() as &dyn Any;
        if let Some(selecting) = any.downcast_ref::<Selecting>() {
            Ok(selecting.total_page(&self.shared))
        } else {
            Err(EditorError::new(EditorErrorKind::InvalidState))
        }
    }
    pub fn select(&mut self, n: usize) -> Result<(), EditorError> {
        let any = self.state.as_mut() as &mut dyn Any;
        let selecting = match any.downcast_mut::<Selecting>() {
            Some(selecting) => selecting,
            None => return Err(EditorError::new(EditorErrorKind::InvalidState)),
        };
        match selecting.select(&mut self.shared, n) {
            Transition::ToState(to_state) => {
                self.shared.last_key_behavior = EditorKeyBehavior::Absorb;
                self.state = to_state;
            }
            Transition::Spin(behavior) => self.shared.last_key_behavior = behavior,
        }
        if self.shared.last_key_behavior == EditorKeyBehavior::Absorb {
            self.shared.try_auto_commit();
        }
        if self.shared.last_key_behavior == EditorKeyBehavior::Bell {
            Err(EditorError::new(EditorErrorKind::InvalidState))
        } else {
            Ok(())
        }
    }
    pub fn cancel_selecting(&mut self) -> Result<(), EditorError> {
        if self.is_selecting() {
            self.shared.cancel_selecting();
            self.state = Box::new(Entering);
            Ok(())
        } else {
            Err(EditorError::new(EditorErrorKind::InvalidState))
        }
    }
    pub fn cancel_entering_syllable(&mut self) {
        self.shared.syl.clear();
        self.state = Box::new(Entering);
    }
    pub fn last_key_behavior(&self) -> EditorKeyBehavior {
        self.shared.last_key_behavior
    }
    pub fn is_entering(&self) -> bool {
        let any = self.state.as_ref() as &dyn Any;
        any.is::<Entering>()
    }
    pub fn is_selecting(&self) -> bool {
        let any = self.state.as_ref() as &dyn Any;
        any.is::<Selecting>()
    }
    pub fn intervals(&self) -> impl Iterator<Item = Interval> {
        self.shared.intervals()
    }
    pub fn len(&self) -> usize {
        self.shared.com.len()
    }
    pub fn is_empty(&self) -> bool {
        self.shared.com.is_empty()
    }
    pub fn hypotheses(&self) -> Vec<Outcome> {
        self.shared.hypotheses()
    }
    /// TODO: doc, rename this to `render`?
    pub fn display(&self) -> String {
        self.shared
            .conversion()
            .into_iter()
            .map(|interval| interval.text)
            .collect::<String>()
    }
    // TODO: decide the return type
    pub fn display_commit(&self) -> &str {
        &self.shared.commit_buffer
    }
    pub fn commit(&mut self) -> Result<(), EditorError> {
        if self.shared.com.is_empty() {
            return Err(EditorError::new(EditorErrorKind::InvalidState));
        }
        self.shared.commit();
        Ok(())
    }
    pub fn has_next_selection_point(&self) -> bool {
        let any = self.state.as_ref() as &dyn Any;
        if let Some(s) = any.downcast_ref::<Selecting>() {
            match &s.sel {
                Selector::Phrase(s) => s.next_selection_point(&self.shared.dict).is_some(),
                Selector::Symbol(_) => false,
                Selector::SpecialSymmbol(_) => false,
            }
        } else {
            false
        }
    }
    pub fn has_prev_selection_point(&self) -> bool {
        let any = self.state.as_ref() as &dyn Any;
        if let Some(s) = any.downcast_ref::<Selecting>() {
            match &s.sel {
                Selector::Phrase(s) => s.prev_selection_point(&self.shared.dict).is_some(),
                Selector::Symbol(_) => false,
                Selector::SpecialSymmbol(_) => false,
            }
        } else {
            false
        }
    }
    pub fn jump_to_next_selection_point(&mut self) -> Result<(), EditorError> {
        let any = self.state.as_mut() as &mut dyn Any;
        if let Some(s) = any.downcast_mut::<Selecting>() {
            match &mut s.sel {
                Selector::Phrase(s) => s.jump_to_next_selection_point(&self.shared.dict),
                _ => Err(EditorError::new(EditorErrorKind::InvalidState)),
            }
        } else {
            Err(EditorError::new(EditorErrorKind::InvalidState))
        }
    }
    pub fn jump_to_prev_selection_point(&mut self) -> Result<(), EditorError> {
        let any = self.state.as_mut() as &mut dyn Any;
        if let Some(s) = any.downcast_mut::<Selecting>() {
            match &mut s.sel {
                Selector::Phrase(s) => s.jump_to_prev_selection_point(&self.shared.dict),
                _ => Err(EditorError::new(EditorErrorKind::InvalidState)),
            }
        } else {
            Err(EditorError::new(EditorErrorKind::InvalidState))
        }
    }
    pub fn jump_to_first_selection_point(&mut self) -> Result<(), EditorError> {
        let any = self.state.as_mut() as &mut dyn Any;
        if let Some(s) = any.downcast_mut::<Selecting>() {
            match &mut s.sel {
                Selector::Phrase(s) => {
                    s.jump_to_first_selection_point(&self.shared.dict);
                    Ok(())
                }
                _ => Err(EditorError::new(EditorErrorKind::InvalidState)),
            }
        } else {
            Err(EditorError::new(EditorErrorKind::InvalidState))
        }
    }
    pub fn jump_to_last_selection_point(&mut self) -> Result<(), EditorError> {
        let any = self.state.as_mut() as &mut dyn Any;
        if let Some(s) = any.downcast_mut::<Selecting>() {
            match &mut s.sel {
                Selector::Phrase(s) => {
                    s.jump_to_last_selection_point(&self.shared.dict);
                    Ok(())
                }
                _ => Err(EditorError::new(EditorErrorKind::InvalidState)),
            }
        } else {
            Err(EditorError::new(EditorErrorKind::InvalidState))
        }
    }
    pub fn start_selecting(&mut self) -> Result<(), EditorError> {
        let any = self.state.as_mut() as &mut dyn Any;
        let transition = if let Some(s) = any.downcast_mut::<Entering>() {
            s.start_selecting(&mut self.shared)
        } else if let Some(s) = any.downcast_mut::<EnteringSyllable>() {
            // Force entering selection
            s.start_selecting(&mut self.shared)
        } else {
            Transition::Spin(EditorKeyBehavior::Bell)
        };
        match transition {
            Transition::ToState(to_state) => {
                self.shared.last_key_behavior = EditorKeyBehavior::Absorb;
                self.state = to_state;
            }
            Transition::Spin(behavior) => self.shared.last_key_behavior = behavior,
        }
        if self.is_selecting() {
            Ok(())
        } else {
            Err(EditorError::new(EditorErrorKind::InvalidState))
        }
    }
    pub fn notification(&self) -> &str {
        &self.shared.notice_buffer
    }
    pub fn flush(&self) {
        if let Some(ud) = self.shared.user_datadir.clone() {
            let user_dict = self.shared.user_dict.clone();
            let hist_dict = self.shared.hist_dict.clone();

            let mut file_options = File::options();
            file_options.create(true).write(true);

            #[cfg(target_family = "unix")]
            {
                use std::os::unix::fs::OpenOptionsExt;
                file_options.mode(0o600);
            }
            let mut hasher = DefaultHasher::new();

            1.hash(&mut hasher);
            let user_dict_path = ud.join("user_dict.csv");
            let temp_path = ud.join(format!("{:x}.tmp", hasher.finish()));
            let temp_file = match file_options.open(&temp_path) {
                Ok(f) => f,
                Err(err) => {
                    error!("Unable to open file: {err}");
                    return;
                }
            };
            if let Err(err) = user_dict.to_writer(BufWriter::new(temp_file)) {
                error!("Unable to write user dict file: {err}");
                return;
            };
            if let Err(err) = fs::rename(&temp_path, &user_dict_path) {
                error!("Unable to write user dict file: {err}");
                return;
            };

            2.hash(&mut hasher);
            let hist_dict_path = ud.join("history_dict.bin");
            let temp_path = ud.join(format!("{:x}.tmp", hasher.finish()));
            let temp_file = match file_options.open(&temp_path) {
                Ok(f) => f,
                Err(err) => {
                    error!("Unable to open file: {err}");
                    return;
                }
            };
            if let Err(err) = hist_dict.to_writer(BufWriter::new(temp_file)) {
                error!("Unable to write user dict file: {err}");
                return;
            };
            if let Err(err) = fs::rename(&temp_path, &hist_dict_path) {
                error!("Unable to write user dict file: {err}");
                return;
            };
        }
    }
}

impl SharedState {
    fn clear(&mut self) {
        self.last_key_behavior = EditorKeyBehavior::Absorb;
        self.com.clear();
        self.syl.clear();
        self.commit_buffer.clear();
        self.notice_buffer.clear();
        self.nth_conversion = 0;
    }
    fn conversion(&self) -> Vec<Interval> {
        let paths = self.conv.convert(self.com.as_ref());
        if paths.is_empty() {
            return vec![];
        }
        paths[self.nth_conversion % paths.len()].intervals.clone()
    }
    fn hypotheses(&self) -> Vec<Outcome> {
        self.conv.convert(self.com.as_ref())
    }
    fn intervals(&self) -> impl DoubleEndedIterator<Item = Interval> + use<> {
        self.conversion().into_iter()
    }
    fn snapshot(&mut self, force: bool, flex: usize) {
        if !force && !self.options.auto_snapshot_selections {
            return;
        }
        let mut should_skip = flex;
        for interval in self.intervals().rev() {
            if should_skip > interval.len() {
                should_skip -= interval.len();
                continue;
            }
            if interval.is_phrase {
                let wid = self.string_table.intern(&interval.text);
                self.com.select(Selection {
                    start: interval.start,
                    end: interval.end,
                    wid,
                });
            }
        }
        self.nth_conversion = 0;
        debug!("snapshot current composition: {}", self.com);
    }
    fn cursor(&self) -> usize {
        self.com.cursor()
    }
    fn learn_phrase_in_range_notify(
        &mut self,
        start: usize,
        end: usize,
    ) -> Result<(), EditorError> {
        let result = self.learn_phrase_in_range_quiet(start, end);
        match &result {
            Ok(phrase) => {
                self.notice_buffer = format!("加入：{phrase}");
                Ok(())
            }
            Err(msg) => {
                msg.clone_into(&mut self.notice_buffer);
                Err(EditorError::new(EditorErrorKind::InvalidState))
            }
        }
    }
    // FIXME enhance user visible reporting
    fn learn_phrase_in_range_quiet(&mut self, start: usize, end: usize) -> Result<String, String> {
        if end > self.com.len() {
            return Err("加詞失敗：字數不符或夾雜符號".to_owned());
        }
        let symbols = self.com.symbols()[start..end].to_vec();
        if symbols.iter().any(Symbol::is_char) {
            return Err("加詞失敗：字數不符或夾雜符號".to_owned());
        }
        let syllables: Vec<Syllable> = symbols
            .iter()
            .map(|s| s.to_syllable().unwrap_or_default())
            .collect();
        // FIXME
        let phrase = self
            .conversion()
            .into_iter()
            .map(|interval| interval.text)
            .collect::<String>()
            .chars()
            .skip(start)
            .take(end - start)
            .collect::<String>();
        if self
            .user_dict
            .lookup(&syllables, LookupStrategy::Standard)
            .into_iter()
            .any(|(wid, _)| self.string_table.get_text(wid).is_some_and(|s| s == phrase))
        {
            return Err(format!("已有：{phrase}"));
        }
        let result = self
            .learn_phrase(&syllables, &phrase)
            .map_err(|_| "加詞失敗：字數不符或夾雜符號".to_owned());
        if result.is_ok() {
            self.dirty_level += 1;
        }
        result.map(|_| phrase)
    }
    fn learn_phrase(&mut self, syllables: &[Syllable], phrase: &str) -> Result<(), EditorError> {
        if syllables.len() != phrase.chars().count() {
            warn!(
                "syllables({:?})[{}] and phrase({})[{}] has different length",
                &syllables,
                syllables.len(),
                &phrase,
                phrase.chars().count()
            );
            return Err(EditorError::new(EditorErrorKind::InvalidState));
        }
        let wid = self.string_table.intern(phrase);
        let phrases = self.user_dict.lookup(syllables, LookupStrategy::Standard);
        if !phrases.iter().any(|p| p.0 == wid) {
            self.user_dict.insert(syllables, phrase);
            return Ok(());
        }
        self.hist_dict.observe(syllables, phrase);
        self.dirty_level += 1;
        Ok(())
    }
    fn unlearn_phrase(&mut self, syllables: &[Syllable], phrase: &str) -> Result<(), EditorError> {
        self.user_dict.remove(syllables, phrase);
        self.hist_dict.remove(syllables, phrase);
        self.dirty_level += 1;
        Ok(())
    }
    fn switch_language_mode(&mut self) {
        self.options = EditorOptions {
            language_mode: match self.options.language_mode {
                LanguageMode::English => LanguageMode::Chinese,
                LanguageMode::Chinese => LanguageMode::English,
            },
            ..self.options
        };
    }
    fn switch_character_form(&mut self) {
        self.options = EditorOptions {
            character_form: match self.options.character_form {
                CharacterForm::Halfwidth => CharacterForm::Fullwidth,
                CharacterForm::Fullwidth => CharacterForm::Halfwidth,
            },
            ..self.options
        };
    }
    fn cancel_selecting(&mut self) {
        self.com.pop_cursor();
    }
    fn commit(&mut self) {
        self.commit_buffer.clear();
        let intervals = self.conversion();
        debug!("commit {}", self.com);
        if !self.options.disable_auto_learn_phrase {
            self.auto_learn(&intervals);
        }
        let output = intervals
            .into_iter()
            .map(|interval| interval.text)
            .collect::<String>();
        self.commit_buffer.push_str(&output);
        self.com.clear();
        self.nth_conversion = 0;
        self.last_key_behavior = EditorKeyBehavior::Commit;
    }
    fn try_auto_commit(&mut self) {
        let len = self.com.len();
        if len <= self.options.auto_commit_threshold {
            return;
        }
        let intervals: Vec<_> = self.intervals().collect();

        let mut remove = 0;
        self.commit_buffer.clear();
        for it in intervals {
            self.commit_buffer.push_str(&it.text);
            remove += it.len();
            if len - remove <= self.options.auto_commit_threshold {
                break;
            }
        }
        self.com.remove_front(remove);
        debug!(
            "buffer has {} symbols left after auto commit",
            self.com.len()
        );
        self.last_key_behavior = EditorKeyBehavior::Commit;
    }
    fn auto_learn(&mut self, intervals: &[Interval]) {
        for (syllables, phrase) in collect_new_phrases(intervals, self.com.symbols()) {
            self.hist_dict.observe(&syllables, &phrase);
            self.dirty_level += 1;
        }
    }
}

fn collect_new_phrases(intervals: &[Interval], symbols: &[Symbol]) -> Vec<(Vec<Syllable>, String)> {
    debug!("intervals {:?}", intervals);
    let mut pending = String::new();
    let mut syllables = Vec::new();
    let mut phrases = vec![];
    let mut collect = |syllables, pending| {
        if !phrases.iter().any(|(_, p)| p == &pending) {
            debug!("autolearn {:?} as {}", &syllables, &pending);
            phrases.push((syllables, pending))
        }
    };
    // Step 1. collect all intervals
    for interval in intervals.iter().filter(|it| it.len() > 1 && it.is_phrase) {
        let syllables = symbols[interval.start..interval.end]
            .iter()
            .map(|s| s.to_syllable().unwrap())
            .collect();
        let pending = interval.text.clone().into_string();
        collect(syllables, pending);
    }
    // Step 2. collect all intervals with length one including break words
    for interval in intervals {
        if interval.is_phrase && interval.len() == 1 && syllables.len() < SyllableVec::MAX_LEN {
            pending.push_str(&interval.text);
            syllables.extend(
                symbols[interval.start..interval.end]
                    .iter()
                    .map(|s| s.to_syllable().unwrap()),
            );
        } else if !pending.is_empty() {
            collect(mem::take(&mut syllables), mem::take(&mut pending));
        }
    }
    if !pending.is_empty() {
        collect(syllables, pending);
    }
    phrases
}

impl BasicEditor for Editor {
    fn process_keyevent(&mut self, key_event: KeyboardEvent) -> EditorKeyBehavior {
        info!("process {}", key_event);
        // reset?
        self.shared.notice_buffer.clear();
        if self.shared.last_key_behavior == EditorKeyBehavior::Commit {
            self.shared.commit_buffer.clear();
        }

        let orig_nth_conv = self.shared.nth_conversion;
        match self.state.next(&mut self.shared, key_event) {
            Transition::ToState(to_state) => {
                self.shared.last_key_behavior = EditorKeyBehavior::Absorb;
                self.state = to_state;
            }
            Transition::Spin(behavior) => self.shared.last_key_behavior = behavior,
        }

        if self.shared.options.conversion_engine == ConversionEngineKind::SimpleEngine
            && self.is_entering()
            && !self.shared.com.is_empty()
        {
            self.shared.commit();
        }

        if self.is_entering() && self.shared.last_key_behavior == EditorKeyBehavior::Absorb {
            self.shared.try_auto_commit();
        }
        if self.shared.nth_conversion > 0 && self.shared.nth_conversion == orig_nth_conv {
            // Force snapshot if the user Tab multiple times and then
            // start typing or move the cursor
            self.shared.snapshot(true, 0);
        } else if self.shared.nth_conversion == 0 {
            // Only try to auto snapshot if we didn't change the conversion candidate
            self.shared.snapshot(false, 5);
        }
        debug!("last_key_behavior = {:?}", self.shared.last_key_behavior);
        debug!("comp: {:?}", &self.shared.com);
        const DIRTY_THRESHOLD: u16 = 0;
        if self.shared.dirty_level > DIRTY_THRESHOLD {
            let _ = self.flush();
            self.shared.dirty_level = 0;
        }
        self.shared.last_key_behavior
    }
}

#[derive(Debug)]
struct Entering;

#[derive(Debug)]
struct EnteringSyllable;

#[derive(Debug)]
struct Selecting {
    page_no: usize,
    action: SelectingAction,
    sel: Selector,
}

#[derive(Debug)]
enum SelectingAction {
    Insert,
    Replace,
}

#[derive(Debug)]
enum Selector {
    Phrase(PhraseSelector),
    Symbol(SymbolSelector),
    SpecialSymmbol(SpecialSymbolSelector),
}

#[derive(Debug)]
struct Highlighting {
    moving_cursor: usize,
}

impl Entering {
    fn start_selecting(&self, editor: &mut SharedState) -> Transition {
        match editor.com.symbol_for_select() {
            Some(symbol) => {
                if symbol.is_syllable() {
                    Transition::ToState(Box::new(Selecting::new_phrase(editor)))
                } else {
                    Transition::ToState(Box::new(Selecting::new_special_symbol(editor, symbol)))
                }
            }
            None => self.spin_ignore(),
        }
    }
    fn start_selecting_or_input_space(&self, editor: &mut SharedState) -> Transition {
        debug!("buffer {}", editor.com);
        match editor.com.symbol_for_select() {
            Some(symbol) => {
                if symbol.is_syllable() {
                    Transition::ToState(Box::new(Selecting::new_phrase(editor)))
                } else {
                    Transition::ToState(Box::new(Selecting::new_special_symbol(editor, symbol)))
                }
            }
            None if editor.com.is_empty() => {
                match editor.options.character_form {
                    CharacterForm::Halfwidth => editor.commit_buffer.push(' '),
                    CharacterForm::Fullwidth => editor.commit_buffer.push('　'),
                }
                self.spin_commit()
            }
            None => self.spin_ignore(),
        }
    }
    fn start_symbol_input(&self, editor: &mut SharedState) -> Transition {
        if editor.sym_sel.is_empty() {
            self.spin_bell()
        } else {
            Transition::ToState(Box::new(Selecting::new_symbol(editor)))
        }
    }
    fn start_enter_syllable(&self) -> Transition {
        Transition::ToState(Box::new(EnteringSyllable))
    }
    fn start_highlighting(&self, start_cursor: usize) -> Transition {
        Transition::ToState(Box::new(Highlighting::new(start_cursor)))
    }
    fn spin_commit(&self) -> Transition {
        Transition::Spin(EditorKeyBehavior::Commit)
    }
}

impl State for Entering {
    fn next(&mut self, shared: &mut SharedState, ev: KeyboardEvent) -> Transition {
        match ev.ksym {
            SYM_BACKSPACE => {
                if shared.com.is_empty() {
                    self.spin_ignore()
                } else {
                    shared.com.remove_before_cursor();
                    self.spin_absorb()
                }
            }
            SYM_CAPSLOCK => {
                shared.switch_language_mode();
                self.spin_absorb()
            }
            code if ev.ksym.is_digit() && ev.is_state_on(KeyState::Control) => {
                let n = code.to_digit().unwrap_or_default() as usize;
                if n == 0 || n == 1 {
                    return self.start_symbol_input(shared);
                }
                let result = match shared.options.user_phrase_add_dir {
                    UserPhraseAddDirection::Forward => {
                        shared.learn_phrase_in_range_notify(shared.cursor(), shared.cursor() + n)
                    }
                    UserPhraseAddDirection::Backward => {
                        if shared.cursor() >= n {
                            shared
                                .learn_phrase_in_range_notify(shared.cursor() - n, shared.cursor())
                        } else {
                            "加詞失敗：字數不符或夾雜符號".clone_into(&mut shared.notice_buffer);
                            return self.spin_bell();
                        }
                    }
                };
                match result {
                    Ok(_) => self.spin_absorb(),
                    Err(_) => self.spin_bell(),
                }
            }
            SYM_RETURN | SYM_ESC | SYM_TAB | SYM_HOME | SYM_END | SYM_LEFT | SYM_RIGHT | SYM_UP
            | SYM_DOWN | SYM_PAGEUP | SYM_PAGEDOWN
                if shared.com.is_empty() =>
            {
                self.spin_ignore()
            }
            SYM_TAB if shared.com.is_end_of_buffer() => {
                shared.nth_conversion += 1;
                self.spin_absorb()
            }
            SYM_TAB => {
                let interval_ends: Vec<_> = shared.conversion().iter().map(|it| it.end).collect();
                if interval_ends.contains(&shared.cursor()) {
                    shared.com.insert_glue();
                } else {
                    shared.com.insert_break();
                }
                self.spin_absorb()
            }
            // DoubleTab => {
            //     // editor.reset_user_break_and_connect_at_cursor();
            //     (EditorKeyBehavior::Absorb, &Entering)
            // }
            SYM_DELETE => {
                if shared.com.is_end_of_buffer() {
                    self.spin_ignore()
                } else {
                    shared.com.remove_after_cursor();
                    self.spin_absorb()
                }
            }
            SYM_HOME => {
                // shared.snapshot();
                shared.com.move_cursor_to_beginning();
                self.spin_absorb()
            }
            SYM_LEFT if ev.is_state_on(KeyState::Shift) => {
                if shared.com.is_beginning_of_buffer() {
                    return self.spin_ignore();
                }
                // shared.snapshot();
                self.start_highlighting(shared.cursor() - 1)
            }
            SYM_RIGHT if ev.is_state_on(KeyState::Shift) => {
                if shared.com.is_end_of_buffer() {
                    return self.spin_ignore();
                }
                // shared.snapshot();
                self.start_highlighting(shared.cursor() + 1)
            }
            SYM_LEFT => {
                // shared.snapshot();
                shared.com.move_cursor_left(1);
                self.spin_absorb()
            }
            SYM_RIGHT => {
                // shared.snapshot();
                shared.com.move_cursor_right(1);
                self.spin_absorb()
            }
            SYM_UP => self.spin_ignore(),
            SYM_SPACE
                if ev.is_state_on(KeyState::Shift)
                    && shared.options.enable_fullwidth_toggle_key =>
            {
                shared.switch_character_form();
                self.spin_absorb()
            }
            SYM_SPACE
                if shared.options.space_is_select_key
                    && shared.options.language_mode == LanguageMode::Chinese =>
            {
                self.start_selecting_or_input_space(shared)
            }
            SYM_DOWN => {
                debug!("buffer {}", shared.com);
                self.start_selecting(shared)
            }
            SYM_END | SYM_PAGEUP | SYM_PAGEDOWN => {
                // shared.snapshot();
                shared.com.move_cursor_to_end();
                self.spin_absorb()
            }
            SYM_RETURN => {
                shared.commit();
                self.spin_commit()
            }
            SYM_ESC => {
                if shared.options.esc_clear_all_buffer && !shared.com.is_empty() {
                    shared.com.clear();
                    self.spin_absorb()
                } else {
                    self.spin_ignore()
                }
            }
            _ if ev.ksym.is_keypad() && ev.is_state_on(KeyState::NumLock) => {
                if shared.com.is_empty() {
                    shared.commit_buffer.clear();
                    shared.commit_buffer.push(ev.ksym.to_unicode());
                    self.spin_commit()
                } else {
                    shared.com.insert(Symbol::from(ev.ksym.to_unicode()));
                    self.spin_absorb()
                }
            }
            _ => {
                if shared.nth_conversion != 0 {
                    shared.snapshot(true, 0);
                }
                match shared.options.language_mode {
                    LanguageMode::Chinese if ev.ksym == SYM_GRAVE && !ev.has_modifiers() => {
                        self.start_symbol_input(shared)
                    }
                    LanguageMode::Chinese if ev.ksym == SYM_SPACE => {
                        match shared.options.character_form {
                            CharacterForm::Halfwidth => {
                                if shared.com.is_empty() {
                                    shared.commit_buffer.clear();
                                    shared.commit_buffer.push(ev.ksym.to_unicode());
                                    self.spin_commit()
                                } else {
                                    shared.com.insert(Symbol::from(ev.ksym.to_unicode()));
                                    self.spin_absorb()
                                }
                            }
                            CharacterForm::Fullwidth => {
                                let char_ = full_width_symbol_input(ev.ksym.to_unicode()).unwrap();
                                if shared.com.is_empty() {
                                    shared.commit_buffer.clear();
                                    shared.commit_buffer.push(char_);
                                    self.spin_commit()
                                } else {
                                    shared.com.insert(Symbol::from(char_));
                                    self.spin_absorb()
                                }
                            }
                        }
                    }
                    LanguageMode::Chinese => {
                        if shared.options.easy_symbol_input && ev.is_state_on(KeyState::Shift) {
                            // Priortize symbol input
                            if let Some(expended) = shared.abbr.find_abbrev(ev.ksym.to_unicode()) {
                                expended
                                    .chars()
                                    .for_each(|ch| shared.com.insert(Symbol::from(ch)));
                                shared.snapshot(false, 0);
                                return self.spin_absorb();
                            }
                        }
                        if !ev.has_modifiers() && KeyBehavior::Absorb == shared.syl.key_press(ev) {
                            return self.start_enter_syllable();
                        }
                        if let Some(symbol) = special_symbol_input(ev.ksym.to_unicode()) {
                            shared.com.insert(Symbol::from(symbol));
                            shared.snapshot(false, 0);
                            return self.spin_absorb();
                        }
                        if ev.ksym.is_unicode() {
                            match shared.options.character_form {
                                CharacterForm::Halfwidth => {
                                    if shared.com.is_empty() {
                                        // FIXME we should ignore these keys if pre-edit is empty
                                        shared.commit_buffer.clear();
                                        shared.commit_buffer.push(ev.ksym.to_unicode());
                                        return self.spin_commit();
                                    } else {
                                        shared.com.insert(Symbol::from(ev.ksym.to_unicode()));
                                        return self.spin_absorb();
                                    }
                                }
                                CharacterForm::Fullwidth => {
                                    let char_ =
                                        full_width_symbol_input(ev.ksym.to_unicode()).unwrap();
                                    if shared.com.is_empty() {
                                        shared.commit_buffer.clear();
                                        shared.commit_buffer.push(char_);
                                        return self.spin_commit();
                                    } else {
                                        shared.com.insert(Symbol::from(char_));
                                        return self.spin_absorb();
                                    }
                                }
                            }
                        }
                        self.spin_bell()
                    }
                    LanguageMode::English => {
                        if !ev.ksym.is_unicode() {
                            return self.spin_bell();
                        }
                        match shared.options.character_form {
                            CharacterForm::Halfwidth => {
                                if shared.com.is_empty() {
                                    // FIXME we should ignore these keys if pre-edit is empty
                                    shared.commit_buffer.clear();
                                    shared.commit_buffer.push(ev.ksym.to_unicode());
                                    self.spin_commit()
                                } else {
                                    shared.com.insert(Symbol::from(ev.ksym.to_unicode()));
                                    self.spin_absorb()
                                }
                            }
                            CharacterForm::Fullwidth => {
                                let char_ = full_width_symbol_input(ev.ksym.to_unicode()).unwrap();
                                if shared.com.is_empty() {
                                    shared.commit_buffer.clear();
                                    shared.commit_buffer.push(char_);
                                    self.spin_commit()
                                } else {
                                    shared.com.insert(Symbol::from(char_));
                                    self.spin_absorb()
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

impl EnteringSyllable {
    fn start_entering(&self) -> Transition {
        Transition::ToState(Box::new(Entering))
    }
    fn start_selecting(&self, editor: &mut SharedState) -> Transition {
        editor.syl.clear();
        match editor.com.symbol_for_select() {
            Some(symbol) => {
                if symbol.is_syllable() {
                    Transition::ToState(Box::new(Selecting::new_phrase(editor)))
                } else {
                    Transition::ToState(Box::new(Selecting::new_special_symbol(editor, symbol)))
                }
            }
            None => self.spin_ignore(),
        }
    }
    fn start_selecting_simple_engine(&self, editor: &mut SharedState) -> Transition {
        editor.syl.clear();
        Transition::ToState(Box::new(Selecting::new_phrase_for_simple_engine(editor)))
    }
}

impl State for EnteringSyllable {
    fn next(&mut self, shared: &mut SharedState, ev: KeyboardEvent) -> Transition {
        match ev.ksym {
            SYM_BACKSPACE => {
                shared.syl.remove_last();

                if !shared.syl.is_empty() {
                    self.spin_absorb()
                } else {
                    self.start_entering()
                }
            }
            SYM_CAPSLOCK => {
                shared.syl.clear();
                shared.switch_language_mode();
                self.start_entering()
            }
            SYM_ESC => {
                shared.syl.clear();
                if shared.options.esc_clear_all_buffer {
                    shared.com.clear();
                }
                self.start_entering()
            }
            _ => {
                let lookup_strategy = match shared.options.conversion_engine {
                    ConversionEngineKind::ChewingEngine | ConversionEngineKind::SimpleEngine => {
                        LookupStrategy::Standard
                    }
                    ConversionEngineKind::FuzzyChewingEngine => LookupStrategy::FuzzyPartialPrefix,
                };
                let key_behavior = match lookup_strategy {
                    LookupStrategy::FuzzyPartialPrefix => shared.syl.fuzzy_key_press(ev),
                    LookupStrategy::Standard => shared.syl.key_press(ev),
                };
                match key_behavior {
                    KeyBehavior::Absorb => self.spin_absorb(),
                    KeyBehavior::Fuzzy(syl) => {
                        if !shared.dict.lookup(&[syl], lookup_strategy).is_empty() {
                            shared.com.insert(Symbol::from(syl));
                        }
                        self.spin_absorb()
                    }
                    KeyBehavior::Commit => {
                        if !shared
                            .dict
                            .lookup(&[shared.syl.read()], lookup_strategy)
                            .is_empty()
                        {
                            shared.com.insert(Symbol::from(shared.syl.read()));
                            shared.syl.clear();
                            if shared.options.conversion_engine
                                == ConversionEngineKind::SimpleEngine
                            {
                                self.start_selecting_simple_engine(shared)
                            } else {
                                self.start_entering()
                            }
                        } else {
                            shared.syl.clear();
                            self.start_entering()
                        }
                    }
                    _ => self.spin_bell(),
                }
            }
        }
    }
}

impl Selecting {
    fn new_phrase(editor: &mut SharedState) -> Self {
        editor.com.push_cursor();
        editor.com.clamp_cursor();

        let mut sel = PhraseSelector::new(
            !editor.options.phrase_choice_rearward,
            editor.options.conversion_engine,
            editor.com.to_composition(),
        );
        sel.init(editor.cursor(), &editor.dict);

        Selecting {
            page_no: 0,
            action: SelectingAction::Replace,
            sel: Selector::Phrase(sel),
        }
    }
    fn new_phrase_for_simple_engine(editor: &mut SharedState) -> Self {
        editor.com.push_cursor();
        // editor.com.clamp_cursor();

        let mut sel = PhraseSelector::new(
            false,
            editor.options.conversion_engine,
            editor.com.to_composition(),
        );
        sel.init_single_word(editor.cursor());

        Selecting {
            page_no: 0,
            action: SelectingAction::Replace,
            sel: Selector::Phrase(sel),
        }
    }
    fn new_symbol(editor: &mut SharedState) -> Self {
        Selecting {
            page_no: 0,
            action: SelectingAction::Insert,
            sel: Selector::Symbol(editor.sym_sel.clone()),
        }
    }
    fn new_special_symbol(editor: &mut SharedState, symbol: Symbol) -> Self {
        editor.com.push_cursor();
        editor.com.clamp_cursor();

        let sel = SpecialSymbolSelector::new(symbol);
        if sel.menu().is_empty() {
            // If there's no special symbol then fallback to dynamic symbol table
            let mut sel = Self::new_symbol(editor);
            sel.action = SelectingAction::Replace;
            sel
        } else {
            Selecting {
                page_no: 0,
                action: SelectingAction::Replace,
                sel: Selector::SpecialSymmbol(sel),
            }
        }
    }
    fn candidates(&self, editor: &SharedState) -> Vec<String> {
        let res = match &self.sel {
            Selector::Phrase(sel) => sel
                .candidates(editor)
                .into_iter()
                .filter_map(|wid| editor.string_table.get_text(wid))
                .map(|s| s.into())
                .collect(),
            Selector::Symbol(sel) => sel.menu(),
            Selector::SpecialSymmbol(sel) => sel.menu(),
        };
        debug!("show candidates: {res:?}");
        res
    }
    fn total_page(&self, editor: &SharedState) -> usize {
        self.candidates(editor)
            .len()
            .div_ceil(editor.options.candidates_per_page)
    }
    fn select(&mut self, editor: &mut SharedState, n: usize) -> Transition {
        let offset = self.page_no * editor.options.candidates_per_page + n;
        match self.sel {
            Selector::Phrase(ref sel) => {
                let candidates = sel.candidates(editor);
                match candidates.get(offset) {
                    Some(wid) => {
                        let selection = Selection {
                            start: sel.begin(),
                            end: sel.end(),
                            wid: *wid,
                        };
                        let len = selection.len();
                        editor.com.select(selection);
                        debug!("Auto Shift {}", editor.options.auto_shift_cursor);
                        editor.com.pop_cursor();
                        if editor.options.auto_shift_cursor {
                            if editor.options.phrase_choice_rearward {
                                editor.com.move_cursor_right(1);
                            } else {
                                editor.com.move_cursor_right(len);
                            }
                        }
                        self.start_entering()
                    }
                    None => self.spin_bell(),
                }
            }
            Selector::Symbol(ref mut sel) => match sel.select(offset) {
                Some(s) => {
                    match self.action {
                        SelectingAction::Insert => editor.com.insert(s),
                        SelectingAction::Replace => editor.com.replace(s),
                    }
                    editor.com.pop_cursor();
                    self.start_entering()
                }
                None => {
                    self.page_no = 0;
                    self.spin_absorb()
                }
            },
            Selector::SpecialSymmbol(ref sel) => match sel.select(offset) {
                Some(s) => {
                    match self.action {
                        SelectingAction::Insert => editor.com.insert(s),
                        SelectingAction::Replace => editor.com.replace(s),
                    }
                    editor.com.pop_cursor();
                    self.start_entering()
                }
                None => {
                    self.page_no = 0;
                    self.spin_absorb()
                }
            },
        }
    }
    fn start_entering(&self) -> Transition {
        Transition::ToState(Box::new(Entering))
    }
}

impl State for Selecting {
    fn next(&mut self, shared: &mut SharedState, ev: KeyboardEvent) -> Transition {
        if ev.is_state_on(KeyState::Control) || ev.is_state_on(KeyState::Shift) {
            return self.spin_bell();
        }

        match ev.ksym {
            SYM_BACKSPACE => {
                shared.cancel_selecting();
                self.start_entering()
            }
            SYM_CAPSLOCK => {
                shared.switch_language_mode();
                shared.cancel_selecting();
                self.start_entering()
            }
            SYM_UP => {
                shared.cancel_selecting();
                self.start_entering()
            }
            SYM_DOWN | SYM_SPACE => {
                if self.page_no + 1 < self.total_page(shared) {
                    self.page_no += 1;
                } else {
                    self.page_no = 0;
                    match &mut self.sel {
                        Selector::Phrase(sel) => {
                            sel.next(&shared.dict);
                        }
                        Selector::Symbol(_sel) => (),
                        Selector::SpecialSymmbol(_sel) => (),
                    }
                }
                self.spin_absorb()
            }
            SYM_LOWER_J => {
                if shared.com.is_empty() {
                    return self.spin_ignore();
                }
                let begin = match &self.sel {
                    Selector::Phrase(sel) => sel.begin(),
                    Selector::Symbol(_) => shared.com.cursor(),
                    Selector::SpecialSymmbol(_) => shared.com.cursor(),
                };
                shared.com.move_cursor(begin.saturating_sub(1));
                let sym = shared.com.symbol().expect("should have symbol");
                if sym.is_syllable() {
                    let mut sel = PhraseSelector::new(
                        !shared.options.phrase_choice_rearward,
                        shared.options.conversion_engine,
                        shared.com.to_composition(),
                    );
                    sel.init(shared.cursor(), &shared.dict);
                    self.sel = Selector::Phrase(sel);
                } else {
                    let sel = SpecialSymbolSelector::new(sym);
                    self.sel = Selector::SpecialSymmbol(sel);
                }
                self.spin_absorb()
            }
            SYM_LOWER_K => {
                if shared.com.is_empty() {
                    return self.spin_ignore();
                }
                let begin = match &self.sel {
                    Selector::Phrase(sel) => sel.begin(),
                    Selector::Symbol(_) => shared.com.cursor(),
                    Selector::SpecialSymmbol(_) => shared.com.cursor(),
                };
                shared.com.move_cursor(begin.saturating_add(1));
                shared.com.clamp_cursor();
                let sym = shared.com.symbol().expect("should have symbol");
                if sym.is_syllable() {
                    let mut sel = PhraseSelector::new(
                        !shared.options.phrase_choice_rearward,
                        shared.options.conversion_engine,
                        shared.com.to_composition(),
                    );
                    sel.init(shared.cursor(), &shared.dict);
                    self.sel = Selector::Phrase(sel);
                } else {
                    let sel = SpecialSymbolSelector::new(sym);
                    self.sel = Selector::SpecialSymmbol(sel);
                }
                self.spin_absorb()
            }
            SYM_LEFT | SYM_PAGEUP => {
                if self.page_no > 0 {
                    self.page_no -= 1;
                } else {
                    self.page_no = self.total_page(shared).saturating_sub(1);
                }
                self.spin_absorb()
            }
            SYM_RIGHT | SYM_PAGEDOWN => {
                if self.page_no + 1 < self.total_page(shared) {
                    self.page_no += 1;
                } else {
                    self.page_no = 0;
                }
                self.spin_absorb()
            }
            _ if ev.ksym.is_digit() => {
                let n = ev.ksym.to_digit().unwrap() as usize;
                let n = if n == 0 { 9 } else { n - 1 };
                self.select(shared, n)
            }
            SYM_ESC => {
                shared.cancel_selecting();
                shared.com.pop_cursor();
                if shared.options.conversion_engine == ConversionEngineKind::SimpleEngine {
                    shared.com.clear();
                }
                self.start_entering()
            }
            SYM_DELETE => {
                // NB: should be Ignore but return Absorb for backward compat
                self.spin_absorb()
            }
            _ => self.spin_bell(),
        }
    }
}

impl Highlighting {
    fn new(moving_cursor: usize) -> Self {
        Highlighting { moving_cursor }
    }
    fn start_entering(&self) -> Transition {
        Transition::ToState(Box::new(Entering))
    }
}

impl State for Highlighting {
    fn next(&mut self, shared: &mut SharedState, ev: KeyboardEvent) -> Transition {
        match ev.ksym {
            SYM_CAPSLOCK => {
                shared.switch_language_mode();
                self.start_entering()
            }
            SYM_LEFT if ev.is_state_on(KeyState::Shift) => {
                if self.moving_cursor != 0 {
                    self.moving_cursor -= 1;
                }
                self.spin_absorb()
            }
            SYM_RIGHT if ev.is_state_on(KeyState::Shift) => {
                if self.moving_cursor != shared.com.len() {
                    self.moving_cursor += 1;
                }
                self.spin_absorb()
            }
            SYM_RETURN => {
                let start = min(self.moving_cursor, shared.com.cursor());
                let end = max(self.moving_cursor, shared.com.cursor());
                shared.com.move_cursor(self.moving_cursor);
                let _ = shared.learn_phrase_in_range_notify(start, end);
                self.start_entering()
            }
            _ => self.start_entering(),
        }
    }
}

#[derive(Debug)]
pub struct EditorBuilder {
    string_table: StringTable,
    static_dict: StaticDict,
    rare_dict: StaticDict,
    user_dict: UserDict,
    history_dict: HistoryDict,
    lm: StaticLm,
    abbrev: AbbrevTable,
    sym_sel: SymbolSelector,
    lookup_strategy: LookupStrategy,
}

impl EditorBuilder {
    pub fn new() -> Self {
        let string_table = StringTable::new();
        let user_dict = UserDict::new(string_table.clone());
        let history_dict = HistoryDict::new(string_table.clone());

        Self {
            string_table,
            static_dict: StaticDict::new(),
            rare_dict: StaticDict::new(),
            user_dict,
            history_dict,
            lm: StaticLm::new(),
            abbrev: AbbrevTable::new(),
            sym_sel: SymbolSelector::default(),
            lookup_strategy: LookupStrategy::Standard,
        }
    }

    pub fn string_table(mut self, st: StringTable) -> Self {
        self.string_table = st;
        self
    }

    pub fn static_dict(mut self, d: StaticDict) -> Self {
        self.static_dict = d;
        self
    }

    pub fn rare_dict(mut self, d: StaticDict) -> Self {
        self.rare_dict = d;
        self
    }

    pub fn user_dict(mut self, d: UserDict) -> Self {
        self.user_dict = d;
        self
    }

    pub fn history_dict(mut self, d: HistoryDict) -> Self {
        self.history_dict = d;
        self
    }

    pub fn static_lm(mut self, lm: StaticLm) -> Self {
        self.lm = lm;
        self
    }

    pub fn lookup_strategy(mut self, s: LookupStrategy) -> Self {
        self.lookup_strategy = s;
        self
    }

    pub fn build(self) -> Editor {
        let dict = CompositeDict::new(
            self.static_dict.clone(),
            self.rare_dict.clone(),
            self.history_dict.clone(),
            self.user_dict.clone(),
        );

        let word_lattice_builder = LatticeBuilder {
            dict: dict.clone(),
            lookup_strategy: self.lookup_strategy,
        };

        let decoder = Decoder {
            lm: self.lm,
            alpha: Decoder::ALPHA,
        };
        let conversion_engine = Box::new(ChewingEngine {
            word_lattice_builder,
            decoder: decoder.clone(),
            string_table: self.string_table.clone(),
        });

        Editor::new(
            None,
            conversion_engine,
            self.string_table,
            dict,
            self.user_dict,
            self.history_dict,
            decoder,
            self.abbrev,
            self.sym_sel,
        )
    }
}

/// All different errors that may happen when changing editor state.
#[derive(Debug)]
pub enum EditorErrorKind {
    /// Requested invalid state change.
    InvalidState,
    /// Requested invalid input.
    InvalidInput,
    /// Requested state change was not possible.
    Impossible,
}

#[derive(Debug)]
pub struct EditorError {
    kind: EditorErrorKind,
    source: Option<Box<dyn Error + Send + Sync + 'static>>,
}

impl EditorError {
    fn new(kind: EditorErrorKind) -> EditorError {
        EditorError { kind, source: None }
    }
}

impl Display for EditorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Editor cannot perform requested action: {:?}", self.kind)
    }
}

impl_exn!(EditorError);
impl_context_error!(pub NewEditorError);

#[cfg(test)]
mod tests {
    use super::BasicEditor;
    use super::collect_new_phrases;
    use crate::dictionary::StringTableBuilder;
    use crate::editor::{EditorBuilder, LanguageMode};
    use crate::lm::StaticDictBuilder;
    use crate::user::UserDict;
    use crate::{
        conversion::{Interval, Symbol},
        editor::EditorKeyBehavior,
        input::{
            KeyboardEvent, keycode,
            keymap::{QWERTY_MAP, map_ascii},
            keysym,
        },
        syl,
        zhuyin::Bopomofo as bpmf,
    };

    const CAPSLOCK_EVENT: KeyboardEvent = KeyboardEvent::builder()
        .code(keycode::KEY_CAPSLOCK)
        .ksym(keysym::SYM_CAPSLOCK)
        .caps_lock_if(true)
        .build();

    #[test]
    fn editing_mode_input_bopomofo() {
        let mut editor = EditorBuilder::new().build();

        let ev = KeyboardEvent {
            code: keycode::KEY_H,
            ksym: keysym::SYM_LOWER_H,
            state: 0,
        };
        let key_behavior = editor.process_keyevent(ev);

        assert_eq!(EditorKeyBehavior::Absorb, key_behavior);
        assert_eq!(syl![bpmf::C], editor.syllable_buffer());

        let ev = KeyboardEvent {
            code: keycode::KEY_K,
            ksym: keysym::SYM_LOWER_K,
            state: 0,
        };
        let key_behavior = editor.process_keyevent(ev);

        assert_eq!(EditorKeyBehavior::Absorb, key_behavior);
        assert_eq!(syl![bpmf::C, bpmf::E], editor.syllable_buffer());
    }

    #[test]
    fn editing_mode_input_bopomofo_commit() {
        let mut builder = StringTableBuilder::new();
        builder.insert("冊");
        let string_table = builder.build();
        let mut dict_builder = StaticDictBuilder::new();
        dict_builder.insert(
            &[syl![bpmf::C, bpmf::E, bpmf::TONE4]],
            string_table.get_wid("冊").unwrap(),
        );
        let dict = dict_builder.build();
        let mut editor = EditorBuilder::new()
            .string_table(string_table)
            .static_dict(dict)
            .build();

        let keys = [b'h', b'k', b'4'];
        let key_behaviors: Vec<_> = keys
            .into_iter()
            .map(|key| map_ascii(&QWERTY_MAP, key))
            .map(|ev| editor.process_keyevent(ev))
            .collect();

        assert_eq!(
            vec![
                EditorKeyBehavior::Absorb,
                EditorKeyBehavior::Absorb,
                EditorKeyBehavior::Absorb
            ],
            key_behaviors
        );
        assert!(editor.syllable_buffer().is_empty());
        assert_eq!("冊", editor.display());
    }

    #[test]
    fn editing_mode_input_bopomofo_select() {
        let mut builder = StringTableBuilder::new();
        builder.insert("冊");
        builder.insert("測");
        let string_table = builder.build();
        let mut dict_builder = StaticDictBuilder::new();
        dict_builder.insert(
            &[syl![bpmf::C, bpmf::E, bpmf::TONE4]],
            string_table.get_wid("冊").unwrap(),
        );
        dict_builder.insert(
            &[syl![bpmf::C, bpmf::E, bpmf::TONE4]],
            string_table.get_wid("測").unwrap(),
        );
        let dict = dict_builder.build();
        let user_dict = UserDict::new(string_table.clone());
        user_dict.boost(&[syl![bpmf::C, bpmf::E, bpmf::TONE4]], "測");
        let mut editor = EditorBuilder::new()
            .string_table(string_table)
            .static_dict(dict)
            .user_dict(user_dict)
            .build();

        editor.set_editor_options(|opt| opt.sort_candidates_by_frequency = false);

        editor.process_keyevent(
            KeyboardEvent::builder()
                .code(keycode::KEY_H)
                .ksym(keysym::SYM_LOWER_H)
                .build(),
        );
        editor.process_keyevent(
            KeyboardEvent::builder()
                .code(keycode::KEY_K)
                .ksym(keysym::SYM_LOWER_H)
                .build(),
        );
        editor.process_keyevent(
            KeyboardEvent::builder()
                .code(keycode::KEY_4)
                .ksym(keysym::SYM_4)
                .build(),
        );
        editor.process_keyevent(
            KeyboardEvent::builder()
                .code(keycode::KEY_DOWN)
                .ksym(keysym::SYM_DOWN)
                .build(),
        );
        let candidates = editor
            .all_candidates()
            .expect("should be in selection mode");
        assert_eq!(vec!["冊", "測"], candidates);
    }

    #[test]
    fn editing_mode_input_bopomofo_select_sorted() {
        let mut builder = StringTableBuilder::new();
        builder.insert("冊");
        builder.insert("測");
        let string_table = builder.build();
        let mut dict_builder = StaticDictBuilder::new();
        dict_builder.insert(
            &[syl![bpmf::C, bpmf::E, bpmf::TONE4]],
            string_table.get_wid("冊").unwrap(),
        );
        dict_builder.insert(
            &[syl![bpmf::C, bpmf::E, bpmf::TONE4]],
            string_table.get_wid("測").unwrap(),
        );
        let dict = dict_builder.build();
        let user_dict = UserDict::new(string_table.clone());
        user_dict.boost(&[syl![bpmf::C, bpmf::E, bpmf::TONE4]], "測");
        let mut editor = EditorBuilder::new()
            .string_table(string_table)
            .static_dict(dict)
            .user_dict(user_dict)
            .build();

        editor.set_editor_options(|opt| opt.sort_candidates_by_frequency = true);

        editor.process_keyevent(
            KeyboardEvent::builder()
                .code(keycode::KEY_H)
                .ksym(keysym::SYM_LOWER_H)
                .build(),
        );
        editor.process_keyevent(
            KeyboardEvent::builder()
                .code(keycode::KEY_K)
                .ksym(keysym::SYM_LOWER_H)
                .build(),
        );
        editor.process_keyevent(
            KeyboardEvent::builder()
                .code(keycode::KEY_4)
                .ksym(keysym::SYM_4)
                .build(),
        );
        editor.process_keyevent(
            KeyboardEvent::builder()
                .code(keycode::KEY_DOWN)
                .ksym(keysym::SYM_DOWN)
                .build(),
        );
        let candidates = editor
            .all_candidates()
            .expect("should be in selection mode");
        assert_eq!(vec!["測", "冊"], candidates);
    }

    #[test]
    fn editing_mode_input_chinese_to_english_mode() {
        let mut builder = StringTableBuilder::new();
        builder.insert("冊");
        builder.insert("測");
        let string_table = builder.build();
        let mut dict_builder = StaticDictBuilder::new();
        dict_builder.insert(
            &[syl![bpmf::C, bpmf::E, bpmf::TONE4]],
            string_table.get_wid("冊").unwrap(),
        );
        let dict = dict_builder.build();
        let mut editor = EditorBuilder::new()
            .string_table(string_table)
            .static_dict(dict)
            .build();

        let keys = [
            map_ascii(&QWERTY_MAP, b'h'),
            map_ascii(&QWERTY_MAP, b'k'),
            map_ascii(&QWERTY_MAP, b'4'),
            // Toggle english mode
            CAPSLOCK_EVENT,
            map_ascii(&QWERTY_MAP, b'z'),
        ];

        let key_behaviors: Vec<_> = keys
            .iter()
            .map(|&key| editor.process_keyevent(key))
            .collect();

        assert_eq!(
            vec![
                EditorKeyBehavior::Absorb,
                EditorKeyBehavior::Absorb,
                EditorKeyBehavior::Absorb,
                EditorKeyBehavior::Absorb,
                EditorKeyBehavior::Absorb,
            ],
            key_behaviors
        );
        assert!(editor.syllable_buffer().is_empty());
        assert_eq!("冊z", editor.display());
    }

    #[test]
    fn editing_mode_input_english_to_chinese_mode() {
        let mut builder = StringTableBuilder::new();
        builder.insert("冊");
        builder.insert("測");
        let string_table = builder.build();
        let mut dict_builder = StaticDictBuilder::new();
        dict_builder.insert(
            &[syl![bpmf::C, bpmf::E, bpmf::TONE4]],
            string_table.get_wid("冊").unwrap(),
        );
        let dict = dict_builder.build();
        let mut editor = EditorBuilder::new()
            .string_table(string_table)
            .static_dict(dict)
            .build();

        let keys = [
            // Switch to english mode
            CAPSLOCK_EVENT,
            map_ascii(&QWERTY_MAP, b'x'),
        ];

        let key_behaviors: Vec<_> = keys
            .iter()
            .map(|&key| editor.process_keyevent(key))
            .collect();

        assert_eq!(
            vec![EditorKeyBehavior::Absorb, EditorKeyBehavior::Commit],
            key_behaviors
        );
        assert!(editor.syllable_buffer().is_empty());
        assert_eq!("x", editor.display_commit());

        let keys = [
            // Switch to chinese mode
            CAPSLOCK_EVENT,
            map_ascii(&QWERTY_MAP, b'h'),
            map_ascii(&QWERTY_MAP, b'k'),
            map_ascii(&QWERTY_MAP, b'4'),
        ];

        let key_behaviors: Vec<_> = keys
            .iter()
            .map(|&key| editor.process_keyevent(key))
            .collect();

        assert_eq!(
            vec![
                EditorKeyBehavior::Absorb,
                EditorKeyBehavior::Absorb,
                EditorKeyBehavior::Absorb,
                EditorKeyBehavior::Absorb,
            ],
            key_behaviors
        );
        assert!(editor.syllable_buffer().is_empty());
        assert_eq!("冊", editor.display());
    }

    #[test]
    fn editing_mode_input_switch_mode_behavior() {
        let mut editor = EditorBuilder::new().build();

        editor.set_editor_options(|opt| opt.language_mode = LanguageMode::English);

        editor.process_keyevent(map_ascii(&QWERTY_MAP, b'X'));

        editor.set_editor_options(|opt| opt.language_mode = LanguageMode::Chinese);

        assert_eq!(EditorKeyBehavior::Commit, editor.last_key_behavior());
        assert_eq!("X", editor.display_commit());
    }

    #[test]
    fn editing_chinese_mode_input_special_symbol() {
        let mut builder = StringTableBuilder::new();
        builder.insert("冊");
        let string_table = builder.build();
        let mut dict_builder = StaticDictBuilder::new();
        dict_builder.insert(
            &[syl![bpmf::C, bpmf::E, bpmf::TONE4]],
            string_table.get_wid("冊").unwrap(),
        );
        let dict = dict_builder.build();
        let mut editor = EditorBuilder::new()
            .string_table(string_table)
            .static_dict(dict)
            .build();

        let keys = [
            map_ascii(&QWERTY_MAP, b'!'),
            map_ascii(&QWERTY_MAP, b'h'),
            map_ascii(&QWERTY_MAP, b'k'),
            map_ascii(&QWERTY_MAP, b'4'),
            map_ascii(&QWERTY_MAP, b'<'),
        ];

        let key_behaviors: Vec<_> = keys
            .iter()
            .map(|&key| editor.process_keyevent(key))
            .collect();

        assert_eq!(
            vec![
                EditorKeyBehavior::Absorb,
                EditorKeyBehavior::Absorb,
                EditorKeyBehavior::Absorb,
                EditorKeyBehavior::Absorb,
                EditorKeyBehavior::Absorb,
            ],
            key_behaviors
        );
        assert!(editor.syllable_buffer().is_empty());
        assert_eq!("！冊，", editor.display());
    }

    #[test]
    fn editing_mode_input_full_shape_symbol() {
        let mut editor = EditorBuilder::new().build();

        editor.shared.switch_character_form();

        let steps = [
            (CAPSLOCK_EVENT, EditorKeyBehavior::Absorb, "", "", ""),
            (
                map_ascii(&QWERTY_MAP, b'0'),
                EditorKeyBehavior::Commit,
                "",
                "",
                "０",
            ),
            (
                map_ascii(&QWERTY_MAP, b'-'),
                EditorKeyBehavior::Commit,
                "",
                "",
                "－",
            ),
        ];

        for s in steps {
            let key = s.0;
            let kb = editor.process_keyevent(key);
            assert_eq!(s.1, kb);
            assert_eq!(s.2, editor.syllable_buffer().to_string());
            assert_eq!(s.3, editor.display());
            assert_eq!(s.4, editor.display_commit());
        }
    }

    #[test]
    fn editing_mode_open_empty_symbol_table_then_bell() {
        let mut editor = EditorBuilder::new().build();

        let ev = map_ascii(&QWERTY_MAP, b'`');
        let key_behavior = editor.process_keyevent(ev);

        assert_eq!(EditorKeyBehavior::Bell, key_behavior);
        assert_eq!(syl![], editor.syllable_buffer());
    }

    #[test]
    fn collect_new_phrases_with_no_break_word() {
        let intervals = [
            Interval {
                start: 0,
                end: 2,
                is_phrase: true,
                text: "今天".into(),
            },
            Interval {
                start: 2,
                end: 4,
                is_phrase: true,
                text: "天氣".into(),
            },
            Interval {
                start: 4,
                end: 6,
                is_phrase: true,
                text: "真好".into(),
            },
        ];
        let symbols = [
            Symbol::Syllable(syl![bpmf::J, bpmf::I, bpmf::EN]),
            Symbol::Syllable(syl![bpmf::T, bpmf::I, bpmf::AN]),
            Symbol::Syllable(syl![bpmf::T, bpmf::I, bpmf::AN]),
            Symbol::Syllable(syl![bpmf::Q, bpmf::I, bpmf::TONE4]),
            Symbol::Syllable(syl![bpmf::ZH, bpmf::EN]),
            Symbol::Syllable(syl![bpmf::H, bpmf::AU, bpmf::TONE3]),
        ];
        let phrases = collect_new_phrases(&intervals, &symbols);
        assert_eq!(
            vec![
                (
                    vec![
                        syl![bpmf::J, bpmf::I, bpmf::EN],
                        syl![bpmf::T, bpmf::I, bpmf::AN],
                    ],
                    "今天".to_string()
                ),
                (
                    vec![
                        syl![bpmf::T, bpmf::I, bpmf::AN],
                        syl![bpmf::Q, bpmf::I, bpmf::TONE4],
                    ],
                    "天氣".to_string()
                ),
                (
                    vec![
                        syl![bpmf::ZH, bpmf::EN],
                        syl![bpmf::H, bpmf::AU, bpmf::TONE3],
                    ],
                    "真好".to_string()
                ),
            ],
            phrases
        );
    }

    #[test]
    fn collect_new_phrases_with_break_word() {
        let intervals = [
            Interval {
                start: 0,
                end: 2,
                is_phrase: true,
                text: "今天".into(),
            },
            Interval {
                start: 2,
                end: 3,
                is_phrase: true,
                text: "也".into(),
            },
            Interval {
                start: 3,
                end: 4,
                is_phrase: true,
                text: "是".into(),
            },
            Interval {
                start: 4,
                end: 7,
                is_phrase: true,
                text: "好天氣".into(),
            },
        ];
        let symbols = [
            Symbol::Syllable(syl![bpmf::J, bpmf::I, bpmf::EN]),
            Symbol::Syllable(syl![bpmf::T, bpmf::I, bpmf::AN]),
            Symbol::Syllable(syl![bpmf::I, bpmf::EH, bpmf::TONE3]),
            Symbol::Syllable(syl![bpmf::SH, bpmf::TONE4]),
            Symbol::Syllable(syl![bpmf::H, bpmf::AU, bpmf::TONE3]),
            Symbol::Syllable(syl![bpmf::T, bpmf::I, bpmf::AN]),
            Symbol::Syllable(syl![bpmf::Q, bpmf::I, bpmf::TONE4]),
        ];
        let phrases = collect_new_phrases(&intervals, &symbols);
        assert_eq!(
            vec![
                (
                    vec![
                        syl![bpmf::J, bpmf::I, bpmf::EN],
                        syl![bpmf::T, bpmf::I, bpmf::AN],
                    ],
                    "今天".to_string()
                ),
                (
                    vec![
                        syl![bpmf::H, bpmf::AU, bpmf::TONE3],
                        syl![bpmf::T, bpmf::I, bpmf::AN],
                        syl![bpmf::Q, bpmf::I, bpmf::TONE4],
                    ],
                    "好天氣".to_string()
                ),
                (
                    vec![
                        syl![bpmf::I, bpmf::EH, bpmf::TONE3],
                        syl![bpmf::SH, bpmf::TONE4]
                    ],
                    "也是".to_string()
                ),
            ],
            phrases
        );
    }
}
