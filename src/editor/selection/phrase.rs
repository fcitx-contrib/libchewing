use std::cmp::min;

use crate::{
    conversion::{Composition, Gap, Interval},
    dictionary::{CompositeDict, LookupStrategy},
    editor::{EditorError, EditorErrorKind, SharedState},
    model::WordId,
    zhuyin::Syllable,
};

#[derive(Debug)]
pub(crate) struct PhraseSelector {
    begin: usize,
    end: usize,
    forward_select: bool,
    orig: usize,
    lookup_strategy: LookupStrategy,
    com: Composition,
}

impl PhraseSelector {
    pub(crate) fn new(
        forward_select: bool,
        lookup_strategy: LookupStrategy,
        com: Composition,
    ) -> PhraseSelector {
        PhraseSelector {
            begin: 0,
            end: com.len(),
            forward_select,
            orig: 0,
            lookup_strategy,
            com,
        }
    }

    pub(crate) fn init(&mut self, cursor: usize, dict: &CompositeDict) {
        self.orig = cursor;
        if self.forward_select {
            self.begin = if cursor == self.com.len() {
                cursor - 1
            } else {
                cursor
            };
            self.end = self.next_break_point(cursor);
        } else {
            self.end = min(cursor + 1, self.com.len());
            self.begin = self.after_previous_break_point(cursor);
        }
        loop {
            let symbols = &self.com.symbols()[self.begin..self.end];
            let syllables: Vec<Syllable> = symbols
                .iter()
                .map(|s| s.to_syllable().unwrap_or_default())
                .collect();
            debug_assert!(
                !syllables.is_empty(),
                "should not enter here if there's no syllable in range"
            );
            if !dict.lookup(&syllables, self.lookup_strategy).is_empty() {
                break;
            }
            if self.forward_select {
                self.end -= 1;
            } else {
                self.begin += 1;
            }
        }
    }

    pub(crate) fn init_single_word(&mut self, cursor: usize) {
        self.orig = cursor;
        self.end = min(cursor, self.com.len());
        self.begin = self.end - 1;
    }

    pub(crate) fn begin(&self) -> usize {
        self.begin
    }

    pub(crate) fn end(&self) -> usize {
        self.end
    }

    pub(crate) fn next_selection_point(&self, dict: &CompositeDict) -> Option<(usize, usize)> {
        let (mut begin, mut end) = (self.begin, self.end);
        loop {
            if self.forward_select {
                end -= 1;
                if begin == end {
                    return None;
                }
            } else {
                begin += 1;
                if begin == end {
                    return None;
                }
            }
            let symbols = &self.com.symbols()[begin..end];
            let syllables: Vec<Syllable> = symbols
                .iter()
                .map(|s| s.to_syllable().unwrap_or_default())
                .collect();
            if !dict.lookup(&syllables, self.lookup_strategy).is_empty() {
                return Some((begin, end));
            }
        }
    }
    pub(crate) fn prev_selection_point(&self, dict: &CompositeDict) -> Option<(usize, usize)> {
        let (mut begin, mut end) = (self.begin, self.end);
        loop {
            if self.forward_select {
                if end == self.com.len() {
                    return None;
                }
                end += 1;
                if end > self.next_break_point(self.orig) {
                    return None;
                }
            } else {
                if begin == 0 {
                    return None;
                }
                begin -= 1;
                if begin < self.after_previous_break_point(self.orig) {
                    return None;
                }
            }
            let symbols = &self.com.symbols()[begin..end];
            let syllables: Vec<Syllable> = symbols
                .iter()
                .map(|s| s.to_syllable().unwrap_or_default())
                .collect();
            if !dict.lookup(&syllables, self.lookup_strategy).is_empty() {
                return Some((begin, end));
            }
        }
    }
    pub(crate) fn jump_to_next_selection_point(
        &mut self,
        dict: &CompositeDict,
    ) -> Result<(), EditorError> {
        if let Some((begin, end)) = self.next_selection_point(dict) {
            self.begin = begin;
            self.end = end;
            Ok(())
        } else {
            Err(EditorError::new(EditorErrorKind::Impossible))
        }
    }
    pub(crate) fn jump_to_prev_selection_point(
        &mut self,
        dict: &CompositeDict,
    ) -> Result<(), EditorError> {
        if let Some((begin, end)) = self.prev_selection_point(dict) {
            self.begin = begin;
            self.end = end;
            Ok(())
        } else {
            Err(EditorError::new(EditorErrorKind::Impossible))
        }
    }
    pub(crate) fn jump_to_first_selection_point(&mut self, dict: &CompositeDict) {
        self.init(self.orig, dict);
    }
    pub(crate) fn jump_to_last_selection_point(&mut self, dict: &CompositeDict) {
        while self.next_selection_point(dict).is_some() {
            let _ = self.jump_to_next_selection_point(dict);
        }
    }

    pub(crate) fn next(&mut self, dict: &CompositeDict) {
        loop {
            if self.forward_select {
                self.end -= 1;
                if self.begin == self.end {
                    self.end = self.next_break_point(self.begin);
                }
            } else {
                self.begin += 1;
                if self.begin == self.end {
                    self.begin -= 1;
                    self.begin = self.after_previous_break_point(self.begin);
                }
            }
            let symbols = &self.com.symbols()[self.begin..self.end];
            let syllables: Vec<Syllable> = symbols
                .iter()
                .map(|s| s.to_syllable().unwrap_or_default())
                .collect();
            if !dict.lookup(&syllables, self.lookup_strategy).is_empty() {
                break;
            }
        }
    }

    fn next_break_point(&self, mut cursor: usize) -> usize {
        loop {
            if self.com.len() == cursor {
                break;
            }
            if let Some(sym) = self.com.symbol(cursor) {
                if !sym.is_syllable() {
                    break;
                }
            }
            cursor += 1;
        }
        cursor
    }

    fn after_previous_break_point(&self, mut cursor: usize) -> usize {
        loop {
            if cursor == 0 {
                return 0;
            }
            if let Some(Gap::Break) = self.com.gap(cursor) {
                break;
            }
            if let Some(sym) = self.com.symbol(cursor - 1) {
                if !sym.is_syllable() {
                    break;
                }
            }
            cursor -= 1;
        }
        cursor
    }

    pub(crate) fn candidates(&self, editor: &SharedState) -> Vec<WordId> {
        let syllables: Vec<Syllable> = self.com.symbols()[self.begin..self.end]
            .iter()
            .map(|s| s.to_syllable().unwrap_or_default())
            .collect();
        let mut candidates = editor
            .dict
            .lookup(&syllables, self.lookup_strategy)
            .into_iter()
            .collect::<Vec<_>>();
        if self.end - self.begin == 1 {
            let alt = editor
                .syl
                .alt_syllables(self.com.symbol(self.begin).unwrap().to_syllable().unwrap());
            for &syl in alt {
                candidates.extend(editor.dict.lookup(&[syl], self.lookup_strategy).into_iter())
            }
        }
        if editor.options.sort_candidates_by_frequency {
            return editor.decoder.rank(candidates);
        }
        candidates.into_iter().map(|(wid, _)| wid).collect()
    }

    pub(crate) fn interval(&self, phrase: impl Into<Box<str>>) -> Interval {
        Interval {
            start: self.begin,
            end: self.end,
            is_phrase: true,
            text: phrase.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PhraseSelector;
    use crate::{
        conversion::{Composition, Symbol},
        dictionary::{CompositeDict, LookupStrategy, StringTable},
        lm::StaticDict,
        syl,
        user::{HistoryDict, UserDict},
        zhuyin::Bopomofo::*,
    };

    fn make_dict(user_dict: UserDict) -> CompositeDict {
        CompositeDict::new(
            StaticDict::new(),
            StaticDict::new(),
            HistoryDict::new(StringTable::new()),
            user_dict,
        )
    }

    #[test]
    fn init_when_cursor_end_of_buffer_syllable() {
        let mut com = Composition::new();
        com.push(Symbol::from(syl![C, E, TONE4]));
        let mut sel = PhraseSelector {
            begin: 0,
            end: 1,
            forward_select: false,
            orig: 0,
            lookup_strategy: LookupStrategy::Standard,
            com,
        };
        let user_dict = UserDict::new(StringTable::new());
        user_dict.insert(&[syl![C, E, TONE4]], "測");
        sel.init(1, &make_dict(user_dict));

        assert_eq!(0, sel.begin);
        assert_eq!(1, sel.end);
    }

    #[test]
    #[should_panic]
    fn init_when_cursor_end_of_buffer_not_syllable() {
        let mut com = Composition::new();
        com.push(Symbol::from(','));
        let mut sel = PhraseSelector {
            begin: 0,
            end: 1,
            forward_select: false,
            orig: 0,
            lookup_strategy: LookupStrategy::Standard,
            com,
        };
        let user_dict = UserDict::new(StringTable::new());
        user_dict.insert(&[syl![C, E, TONE4]], "測");
        sel.init(1, &make_dict(user_dict));
    }

    #[test]
    fn init_forward_select_when_cursor_end_of_buffer_syllable() {
        let mut com = Composition::new();
        com.push(Symbol::from(syl![C, E, TONE4]));
        let mut sel = PhraseSelector {
            begin: 0,
            end: 1,
            forward_select: true,
            orig: 0,
            lookup_strategy: LookupStrategy::Standard,
            com,
        };
        let user_dict = UserDict::new(StringTable::new());
        user_dict.insert(&[syl![C, E, TONE4]], "測");
        sel.init(1, &make_dict(user_dict));

        assert_eq!(0, sel.begin);
        assert_eq!(1, sel.end);
    }

    #[test]
    #[should_panic]
    fn init_forward_select_when_cursor_end_of_buffer_not_syllable() {
        let mut com = Composition::new();
        com.push(Symbol::from(','));
        let mut sel = PhraseSelector {
            begin: 0,
            end: 1,
            forward_select: true,
            orig: 0,
            lookup_strategy: LookupStrategy::Standard,
            com,
        };
        let user_dict = UserDict::new(StringTable::new());
        user_dict.insert(&[syl![C, E, TONE4]], "測");
        sel.init(1, &make_dict(user_dict));
    }

    #[test]
    fn should_stop_at_left_boundary() {
        let mut com = Composition::new();
        for sym in [
            Symbol::from(syl![C, E, TONE4]),
            Symbol::from(syl![C, E, TONE4]),
        ] {
            com.push(sym);
        }
        let sel = PhraseSelector {
            begin: 0,
            end: 2,
            forward_select: false,
            orig: 0,
            lookup_strategy: LookupStrategy::Standard,
            com,
        };

        assert_eq!(0, sel.after_previous_break_point(0));
        assert_eq!(0, sel.after_previous_break_point(1));
        assert_eq!(0, sel.after_previous_break_point(2));
    }

    #[test]
    fn should_stop_after_first_non_syllable() {
        let mut com = Composition::new();
        for sym in [Symbol::from(','), Symbol::from(syl![C, E, TONE4])] {
            com.push(sym);
        }
        let sel = PhraseSelector {
            begin: 0,
            end: 2,
            forward_select: false,
            orig: 0,
            lookup_strategy: LookupStrategy::Standard,
            com,
        };

        assert_eq!(0, sel.after_previous_break_point(0));
        assert_eq!(1, sel.after_previous_break_point(1));
        assert_eq!(1, sel.after_previous_break_point(2));
    }
}
