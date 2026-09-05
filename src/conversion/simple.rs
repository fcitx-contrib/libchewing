use crate::{
    conversion::{Composition, ConversionEngine, Interval, Outcome},
    dictionary::{CompositeDict, LookupStrategy, StringTable},
};

/// Simple engine does not perform any intelligent conversion.
#[derive(Debug)]
pub struct SimpleEngine {
    string_table: StringTable,
    dict: CompositeDict,
}

impl SimpleEngine {
    pub fn convert<'a>(&'a self, comp: &'a Composition) -> Vec<Outcome> {
        let mut intervals = vec![];

        for (i, sym) in comp.symbols().iter().enumerate() {
            if comp
                .selections()
                .iter()
                .any(|selection| selection.intersect_range(i, i + 1))
            {
                continue;
            }
            if sym.is_char() {
                intervals.push(Interval {
                    start: i,
                    end: i + 1,
                    is_phrase: false,
                    text: sym.to_char().unwrap().to_string().into_boxed_str(),
                });
            } else {
                let phrase = self
                    .dict
                    .lookup(&[sym.to_syllable().unwrap()], LookupStrategy::Standard)
                    .first()
                    .cloned();
                let phrase_str = phrase.map_or_else(
                    || sym.to_syllable().unwrap().to_string(),
                    |(wid, _)| {
                        self.string_table
                            .get_text(wid)
                            .unwrap_or("".into())
                            .to_string()
                    },
                );
                intervals.push(Interval {
                    start: i,
                    end: i + 1,
                    is_phrase: true,
                    text: phrase_str.into_boxed_str(),
                })
            }
        }
        intervals.extend(comp.selections().into_iter().filter_map(|s| {
            if let Some(text) = self.string_table.get_text(s.wid) {
                Some(Interval {
                    start: s.start,
                    end: s.end,
                    is_phrase: true,
                    text: text.into_boxed_str(),
                })
            } else {
                None
            }
        }));
        intervals.sort_by_key(|int| int.start);
        vec![Outcome {
            intervals,
            cost: 0.0,
        }]
    }
}

impl ConversionEngine for SimpleEngine {
    fn convert<'a>(&'a self, comp: &'a Composition) -> Vec<Outcome> {
        SimpleEngine::convert(self, comp)
    }
}

#[cfg(test)]
mod tests {
    use super::SimpleEngine;
    use crate::{
        conversion::{Composition, Interval, Outcome, Symbol},
        dictionary::{CompositeDict, StringTable},
        lm::StaticDict,
        syl,
        user::{HistoryDict, UserDict},
        zhuyin::Bopomofo::*,
    };

    fn test_dictionary(string_table: StringTable) -> CompositeDict {
        let user_dict = UserDict::new(string_table.clone());

        user_dict.insert(&[syl![G, U, O, TONE2]], "國");
        user_dict.insert(&[syl![M, I, EN, TONE2]], "民");
        user_dict.insert(&[syl![D, A, TONE4]], "大");
        user_dict.insert(&[syl![H, U, EI, TONE4]], "會");
        user_dict.insert(&[syl![D, AI, TONE4]], "代");
        user_dict.insert(&[syl![B, I, AU, TONE3]], "表");
        user_dict.insert(&[syl![B, I, AU, TONE3]], "錶");
        user_dict.insert(&[syl![G, U, O, TONE2], syl![M, I, EN, TONE2]], "國民");
        user_dict.insert(&[syl![D, A, TONE4], syl![H, U, EI, TONE4]], "大會");
        user_dict.insert(&[syl![D, AI, TONE4], syl![B, I, AU, TONE3]], "代表");
        user_dict.insert(&[syl![D, AI, TONE4], syl![B, I, AU, TONE3]], "戴錶");
        user_dict.insert(&[syl![X, I, EN]], "心");
        user_dict.insert(&[syl![K, U, TONE4], syl![I, EN]], "庫音");
        user_dict.insert(&[syl![X, I, EN], syl![K, U, TONE4], syl![I, EN]], "新酷音");
        user_dict.insert(
            &[syl![C, E, TONE4], syl![SH, TONE4], syl![I, TONE2]],
            "測試儀",
        );
        user_dict.insert(&[syl![C, E, TONE4], syl![SH, TONE4]], "測試");
        user_dict.insert(&[syl![I, TONE2], syl![X, I, A, TONE4]], "一下");
        user_dict.insert(&[syl![X, I, A, TONE4]], "下");
        user_dict.insert(&[syl![H, A]], "哈");
        user_dict.insert(&[syl![H, A], syl![H, A]], "哈哈");

        CompositeDict::new(
            StaticDict::new(),
            StaticDict::new(),
            HistoryDict::new(string_table),
            user_dict,
        )
    }

    #[test]
    fn convert_simple_chinese_composition() {
        let string_table = StringTable::new();
        let dict = test_dictionary(string_table.clone());
        let engine = SimpleEngine { string_table, dict };
        let mut composition = Composition::new();
        for sym in [
            Symbol::from(syl![G, U, O, TONE2]),
            Symbol::from(syl![M, I, EN, TONE2]),
            Symbol::from(syl![D, A, TONE4]),
            Symbol::from(syl![H, U, EI, TONE4]),
            Symbol::from(syl![D, AI, TONE4]),
            Symbol::from(syl![B, I, AU, TONE3]),
        ] {
            composition.push(sym);
        }
        assert_eq!(
            vec![Outcome {
                intervals: vec![
                    Interval {
                        start: 0,
                        end: 1,
                        is_phrase: true,
                        text: "國".into()
                    },
                    Interval {
                        start: 1,
                        end: 2,
                        is_phrase: true,
                        text: "民".into()
                    },
                    Interval {
                        start: 2,
                        end: 3,
                        is_phrase: true,
                        text: "大".into()
                    },
                    Interval {
                        start: 3,
                        end: 4,
                        is_phrase: true,
                        text: "會".into()
                    },
                    Interval {
                        start: 4,
                        end: 5,
                        is_phrase: true,
                        text: "代".into()
                    },
                    Interval {
                        start: 5,
                        end: 6,
                        is_phrase: true,
                        text: "表".into()
                    },
                ],
                cost: 0.0
            }],
            engine.convert(&composition)
        );
    }
}
