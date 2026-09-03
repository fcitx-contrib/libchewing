use std::fmt::Debug;

use super::{Composition, ConversionEngine, Gap, Interval, Outcome};
use crate::{
    conversion::{Decoder, WordLatticeBuilder},
    dictionary::{LookupStrategy, StringTable},
    model::Surface,
};

/// The default Chewing conversion method.
#[derive(Debug)]
pub struct ChewingEngine {
    pub word_lattice_builder: WordLatticeBuilder,
    pub decoder: Decoder,
    pub string_table: StringTable,
    pub lookup_strategy: LookupStrategy,
}

impl ChewingEngine {
    const MAX_OUT: u8 = 10;

    pub(crate) fn convert<'a>(&'a self, com: &'a Composition) -> Vec<Outcome> {
        let lattice = self.word_lattice_builder.to_lattice(com);
        let hypothesis = self.decoder.decoden(&lattice, Self::MAX_OUT);

        hypothesis
            .into_iter()
            .map(|hyp| {
                let cost = hyp.cost;
                let intervals = hyp
                    .edges
                    .into_iter()
                    .map(|edge| Interval {
                        start: edge.start as usize,
                        end: edge.end as usize,
                        is_phrase: matches!(edge.surface, Surface::Word(_)),
                        text: match edge.surface {
                            Surface::Word(wid) => self
                                .string_table
                                .get_text(wid)
                                .unwrap_or("".into())
                                .to_string()
                                .into_boxed_str(),
                            Surface::Char(ch) => ch.to_string().into_boxed_str(),
                            Surface::None => "".to_string().into_boxed_str(),
                        },
                    })
                    .fold(vec![], |acc, interval| glue_fn(com, acc, interval));
                Outcome { intervals, cost }
            })
            .collect()
    }
}

impl ConversionEngine for ChewingEngine {
    fn convert<'a>(&'a self, comp: &'a Composition) -> Vec<Outcome> {
        ChewingEngine::convert(self, comp)
    }
}

fn glue_fn(com: &Composition, mut acc: Vec<Interval>, interval: Interval) -> Vec<Interval> {
    if acc.is_empty() {
        acc.push(interval);
        return acc;
    }
    let last = acc.last().expect("acc should have at least one item");
    if !last.is_phrase || !interval.is_phrase {
        acc.push(interval);
        return acc;
    }
    if let Some(Gap::Glue) = com.gap(last.end) {
        let last = acc.pop().expect("acc should have at least one item");
        let mut phrase = last.text.into_string();
        phrase.push_str(&interval.text);
        acc.push(Interval {
            start: last.start,
            end: interval.end,
            is_phrase: true,
            text: phrase.into_boxed_str(),
        })
    } else {
        acc.push(interval);
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::ChewingEngine;
    use crate::{
        conversion::{
            Composition, Gap, Interval, Outcome, Symbol,
            chewing::{Edge, PossibleInterval, PossiblePath, PossiblePhrase, n_best_distinct},
        },
        dictionary::{Dictionary, Phrase, TrieBuf},
        syl,
        zhuyin::Bopomofo::*,
    };

    fn test_dictionary() -> impl Dictionary {
        TrieBuf::from([
            (vec![syl![G, U, O, TONE2]], vec![("國", 1)]),
            (vec![syl![M, I, EN, TONE2]], vec![("民", 1)]),
            (vec![syl![D, A, TONE4]], vec![("大", 1)]),
            (vec![syl![H, U, EI, TONE4]], vec![("會", 1)]),
            (vec![syl![D, AI, TONE4]], vec![("代", 1)]),
            (vec![syl![B, I, AU, TONE3]], vec![("表", 1), ("錶", 1)]),
            (
                vec![syl![G, U, O, TONE2], syl![M, I, EN, TONE2]],
                vec![("國民", 200)],
            ),
            (
                vec![syl![D, A, TONE4], syl![H, U, EI, TONE4]],
                vec![("大會", 200)],
            ),
            (
                vec![syl![D, AI, TONE4], syl![B, I, AU, TONE3]],
                vec![("代表", 200), ("戴錶", 100)],
            ),
            (vec![syl![X, I, EN]], vec![("心", 1)]),
            (vec![syl![K, U, TONE4], syl![I, EN]], vec![("庫音", 300)]),
            (
                vec![syl![X, I, EN], syl![K, U, TONE4], syl![I, EN]],
                vec![("新酷音", 200)],
            ),
            (
                vec![syl![C, E, TONE4], syl![SH, TONE4], syl![I, TONE2]],
                vec![("測試儀", 42)],
            ),
            (
                vec![syl![C, E, TONE4], syl![SH, TONE4]],
                vec![("測試", 9318)],
            ),
            (
                vec![syl![I, TONE2], syl![X, I, A, TONE4]],
                vec![("一下", 10576)],
            ),
            (vec![syl![X, I, A, TONE4]], vec![("下", 10576)]),
            (vec![syl![H, A]], vec![("哈", 1)]),
            (vec![syl![H, A], syl![H, A]], vec![("哈哈", 1)]),
        ])
    }

    #[test]
    fn simple_shortest_path() {
        let graph = vec![
            vec![
                Edge {
                    start: 0,
                    end: 1,
                    sn: 0,
                    cost: 1.0,
                },
                Edge {
                    start: 0,
                    end: 2,
                    sn: 2,
                    cost: 3.0,
                },
            ],
            vec![Edge {
                start: 1,
                end: 2,
                sn: 1,
                cost: 1.0,
            }],
        ];
        let phrases = vec![
            PossiblePhrase::Phrase(Phrase::new("測", 1), 1.0),
            PossiblePhrase::Phrase(Phrase::new("試", 1), 1.0),
            PossiblePhrase::Phrase(Phrase::new("測試", 3), 1.0),
        ];

        assert_eq!(
            vec![PossiblePath {
                intervals: vec![
                    PossibleInterval {
                        start: 0,
                        end: 1,
                        phrase: PossiblePhrase::Phrase(Phrase::new("測", 1), 1.0),
                    },
                    PossibleInterval {
                        start: 1,
                        end: 2,
                        phrase: PossiblePhrase::Phrase(Phrase::new("試", 1), 1.0),
                    }
                ]
            }],
            n_best_distinct(&graph, 2, &phrases, 1)
        );
    }

    #[test]
    fn multi_edge_shortest_path() {
        let graph = vec![
            vec![
                Edge {
                    start: 0,
                    end: 1,
                    sn: 0,
                    cost: 1.0,
                },
                Edge {
                    start: 0,
                    end: 1,
                    sn: 3,
                    cost: 2.0,
                },
                Edge {
                    start: 0,
                    end: 2,
                    sn: 2,
                    cost: 3.0,
                },
            ],
            vec![Edge {
                start: 1,
                end: 2,
                sn: 1,
                cost: 1.0,
            }],
        ];

        let phrases = vec![
            PossiblePhrase::Phrase(Phrase::new("測", 1), 1.0),
            PossiblePhrase::Phrase(Phrase::new("試", 1), 1.0),
            PossiblePhrase::Phrase(Phrase::new("測試", 3), 1.0),
            PossiblePhrase::Phrase(Phrase::new("策", 2), 1.0),
        ];

        assert_eq!(
            vec![PossiblePath {
                intervals: vec![
                    PossibleInterval {
                        start: 0,
                        end: 1,
                        phrase: PossiblePhrase::Phrase(Phrase::new("測", 1), 1.0),
                    },
                    PossibleInterval {
                        start: 1,
                        end: 2,
                        phrase: PossiblePhrase::Phrase(Phrase::new("試", 1), 1.0),
                    }
                ]
            }],
            n_best_distinct(&graph, 2, &phrases, 1)
        );
    }

    // #[test]
    // fn convert_empty_composition() {
    //     let dict = test_dictionary();
    //     let engine = ChewingEngine::new();
    //     let composition = Composition::new();
    //     assert_eq!(
    //         vec![Outcome::default()],
    //         engine.convert(&dict, &composition)
    //     );
    // }

    // // Some corrupted user dictionary may contain empty length syllables
    // #[test]
    // fn convert_zero_length_entry() {
    //     let mut dict = test_dictionary();
    //     dict.add_phrase(&[], ("", 0).into()).unwrap();
    //     let engine = ChewingEngine::new();
    //     let mut composition = Composition::new();
    //     for sym in [
    //         Symbol::from(syl![C, E, TONE4]),
    //         Symbol::from(syl![SH, TONE4]),
    //     ] {
    //         composition.push(sym);
    //     }
    //     assert_eq!(
    //         vec![Interval {
    //             start: 0,
    //             end: 2,
    //             is_phrase: true,
    //             text: "測試".into()
    //         }],
    //         engine.convert(&dict, &composition)[0].intervals
    //     );
    // }

    // #[test]
    // fn convert_simple_chinese_composition() {
    //     let dict = test_dictionary();
    //     let engine = ChewingEngine::new();
    //     let mut composition = Composition::new();
    //     for sym in [
    //         Symbol::from(syl![G, U, O, TONE2]),
    //         Symbol::from(syl![M, I, EN, TONE2]),
    //         Symbol::from(syl![D, A, TONE4]),
    //         Symbol::from(syl![H, U, EI, TONE4]),
    //         Symbol::from(syl![D, AI, TONE4]),
    //         Symbol::from(syl![B, I, AU, TONE3]),
    //     ] {
    //         composition.push(sym);
    //     }
    //     assert_eq!(
    //         vec![
    //             Interval {
    //                 start: 0,
    //                 end: 2,
    //                 is_phrase: true,
    //                 text: "國民".into()
    //             },
    //             Interval {
    //                 start: 2,
    //                 end: 4,
    //                 is_phrase: true,
    //                 text: "大會".into()
    //             },
    //             Interval {
    //                 start: 4,
    //                 end: 6,
    //                 is_phrase: true,
    //                 text: "代表".into()
    //             },
    //         ],
    //         engine.convert(&dict, &composition)[0].intervals
    //     );
    // }

    // #[test]
    // fn convert_chinese_composition_with_breaks() {
    //     let dict = test_dictionary();
    //     let engine = ChewingEngine::new();
    //     let mut composition = Composition::new();
    //     for sym in [
    //         Symbol::from(syl![G, U, O, TONE2]),
    //         Symbol::from(syl![M, I, EN, TONE2]),
    //         Symbol::from(syl![D, A, TONE4]),
    //         Symbol::from(syl![H, U, EI, TONE4]),
    //         Symbol::from(syl![D, AI, TONE4]),
    //         Symbol::from(syl![B, I, AU, TONE3]),
    //     ] {
    //         composition.push(sym);
    //     }
    //     composition.set_gap(1, Gap::Break);
    //     composition.set_gap(5, Gap::Break);
    //     assert_eq!(
    //         vec![
    //             Interval {
    //                 start: 0,
    //                 end: 1,
    //                 is_phrase: true,
    //                 text: "國".into()
    //             },
    //             Interval {
    //                 start: 1,
    //                 end: 2,
    //                 is_phrase: true,
    //                 text: "民".into()
    //             },
    //             Interval {
    //                 start: 2,
    //                 end: 4,
    //                 is_phrase: true,
    //                 text: "大會".into()
    //             },
    //             Interval {
    //                 start: 4,
    //                 end: 5,
    //                 is_phrase: true,
    //                 text: "代".into()
    //             },
    //             Interval {
    //                 start: 5,
    //                 end: 6,
    //                 is_phrase: true,
    //                 text: "表".into()
    //             },
    //         ],
    //         engine.convert(&dict, &composition)[0].intervals
    //     );
    // }

    // #[test]
    // fn convert_chinese_composition_with_good_selection() {
    //     let dict = test_dictionary();
    //     let engine = ChewingEngine::new();
    //     let mut composition = Composition::new();
    //     for sym in [
    //         Symbol::from(syl![G, U, O, TONE2]),
    //         Symbol::from(syl![M, I, EN, TONE2]),
    //         Symbol::from(syl![D, A, TONE4]),
    //         Symbol::from(syl![H, U, EI, TONE4]),
    //         Symbol::from(syl![D, AI, TONE4]),
    //         Symbol::from(syl![B, I, AU, TONE3]),
    //     ] {
    //         composition.push(sym);
    //     }
    //     composition.push_selection(Interval {
    //         start: 4,
    //         end: 6,
    //         is_phrase: true,
    //         text: "戴錶".into(),
    //     });
    //     assert_eq!(
    //         vec![
    //             Interval {
    //                 start: 0,
    //                 end: 2,
    //                 is_phrase: true,
    //                 text: "國民".into()
    //             },
    //             Interval {
    //                 start: 2,
    //                 end: 4,
    //                 is_phrase: true,
    //                 text: "大會".into()
    //             },
    //             Interval {
    //                 start: 4,
    //                 end: 6,
    //                 is_phrase: true,
    //                 text: "戴錶".into()
    //             },
    //         ],
    //         engine.convert(&dict, &composition)[0].intervals
    //     );
    // }

    // #[test]
    // fn convert_chinese_composition_with_substring_selection() {
    //     let dict = test_dictionary();
    //     let engine = ChewingEngine::new();
    //     let mut composition = Composition::new();
    //     for sym in [
    //         Symbol::from(syl![X, I, EN]),
    //         Symbol::from(syl![K, U, TONE4]),
    //         Symbol::from(syl![I, EN]),
    //     ] {
    //         composition.push(sym);
    //     }
    //     composition.push_selection(Interval {
    //         start: 1,
    //         end: 3,
    //         is_phrase: true,
    //         text: "酷音".into(),
    //     });
    //     assert_eq!(
    //         vec![Interval {
    //             start: 0,
    //             end: 3,
    //             is_phrase: true,
    //             text: "新酷音".into()
    //         }],
    //         engine.convert(&dict, &composition)[0].intervals
    //     );
    // }

    // #[test]
    // fn multiple_single_word_selection() {
    //     let dict = test_dictionary();
    //     let engine = ChewingEngine::new();
    //     let mut composition = Composition::new();
    //     for sym in [
    //         Symbol::from(syl![D, AI, TONE4]),
    //         Symbol::from(syl![B, I, AU, TONE3]),
    //     ] {
    //         composition.push(sym);
    //     }
    //     for interval in [
    //         Interval {
    //             start: 0,
    //             end: 1,
    //             is_phrase: true,
    //             text: "代".into(),
    //         },
    //         Interval {
    //             start: 1,
    //             end: 2,
    //             is_phrase: true,
    //             text: "錶".into(),
    //         },
    //     ] {
    //         composition.push_selection(interval);
    //     }
    //     assert_eq!(
    //         vec![
    //             Interval {
    //                 start: 0,
    //                 end: 1,
    //                 is_phrase: true,
    //                 text: "代".into()
    //             },
    //             Interval {
    //                 start: 1,
    //                 end: 2,
    //                 is_phrase: true,
    //                 text: "錶".into()
    //             }
    //         ],
    //         engine.convert(&dict, &composition)[0].intervals
    //     );
    // }

    // #[test]
    // fn convert_cycle_alternatives() {
    //     let dict = test_dictionary();
    //     let engine = ChewingEngine::new();
    //     let mut composition = Composition::new();
    //     for sym in [
    //         Symbol::from(syl![C, E, TONE4]),
    //         Symbol::from(syl![SH, TONE4]),
    //         Symbol::from(syl![I, TONE2]),
    //         Symbol::from(syl![X, I, A, TONE4]),
    //     ] {
    //         composition.push(sym);
    //     }
    //     assert_eq!(
    //         vec![
    //             Interval {
    //                 start: 0,
    //                 end: 2,
    //                 is_phrase: true,
    //                 text: "測試".into()
    //             },
    //             Interval {
    //                 start: 2,
    //                 end: 4,
    //                 is_phrase: true,
    //                 text: "一下".into()
    //             }
    //         ],
    //         engine.convert(&dict, &composition)[0].intervals
    //     );
    //     assert_eq!(
    //         vec![
    //             Interval {
    //                 start: 0,
    //                 end: 3,
    //                 is_phrase: true,
    //                 text: "測試儀".into()
    //             },
    //             Interval {
    //                 start: 3,
    //                 end: 4,
    //                 is_phrase: true,
    //                 text: "下".into()
    //             }
    //         ],
    //         engine.convert(&dict, &composition)[1].intervals
    //     );
    //     assert_eq!(
    //         Some(vec![
    //             Interval {
    //                 start: 0,
    //                 end: 2,
    //                 is_phrase: true,
    //                 text: "測試".into()
    //             },
    //             Interval {
    //                 start: 2,
    //                 end: 4,
    //                 is_phrase: true,
    //                 text: "一下".into()
    //             }
    //         ]),
    //         engine
    //             .convert(&dict, &composition)
    //             .into_iter()
    //             .cycle()
    //             .nth(2)
    //             .map(|p| p.intervals)
    //     );
    // }

    // #[test]
    // fn convert_collapses_equal_text_resegmentations() {
    //     let dict = test_dictionary();
    //     let engine = ChewingEngine::new();
    //     let mut composition = Composition::new();
    //     for _ in 0..80 {
    //         composition.push(Symbol::from(syl![H, A]));
    //     }
    //     let outcomes = engine.convert(&dict, &composition);
    //     // Every segmentation of 80 ㄏㄚ (e.g. 40x哈哈, 39x哈哈+2x哈, …) renders to
    //     // the identical visible string, so de-duplicating by text leaves exactly
    //     // one candidate instead of dozens of equal-looking re-segmentations.
    //     assert_eq!(1, outcomes.len());
    //     // The cheapest segmentation pairs every ㄏㄚ into 哈哈 -> 40 intervals.
    //     assert_eq!(40, outcomes[0].intervals.len());
    // }
}
