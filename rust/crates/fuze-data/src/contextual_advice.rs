//! Relative choices from comparable historical evidence, never a causal win-rate prediction.
use crate::decision_evidence::{interval95, Rate};
use std::{cmp::Ordering, collections::BTreeMap};

/// Product conservatism, separate from the 200-game display annotation.
pub const MIN_COMPARISON_SAMPLES: u64 = 500;
pub const MIN_DIFFERENCE: f64 = 0.02;

#[derive(Clone, Debug, PartialEq)]
pub struct Combination {
    pub required: Vec<u32>,
    pub rate: Rate,
}

/// The adapter only supplies stage/combination rows with compatible provenance.
/// `current` contains confirmed prerequisites; future rows never enter comparisons.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Candidate {
    pub id: Option<u32>,
    pub score: Option<f64>,
    pub overall: Option<Rate>,
    pub stages: [Option<Rate>; 4],
    pub current: Vec<Combination>,
    pub future: Vec<Combination>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Choice {
    Preferred,
    Optional,
    Conditional,
    NotAdvised,
    #[default]
    Insufficient,
}
impl Choice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preferred => "本组优先",
            Self::Optional => "可选",
            Self::Conditional => "依赖搭配",
            Self::NotAdvised => "当前不建议",
            Self::Insufficient => "证据不足",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StagePattern {
    #[default]
    Unavailable,
    Partial,
    LowSample,
    NoClearOrder,
    Consistent,
    Reversal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StageSummary {
    pub reported: u8,
    pub supported: u8,
    pub pattern: StagePattern,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Reason {
    OverallRank,
    StageAdvantage {
        other: u32,
    },
    ComboAdvantage {
        other: u32,
        required: Vec<u32>,
    },
    StageAlternative {
        other: u32,
    },
    ComboAlternative {
        other: u32,
        required: Vec<u32>,
    },
    FutureOnly,
    PartialOffer,
    ComparableTie,
    ConflictingEvidence,
    #[default]
    MissingEvidence,
}
impl Reason {
    pub fn required(&self) -> Option<&[u32]> {
        match self {
            Self::ComboAdvantage { required, .. } | Self::ComboAlternative { required, .. } => {
                Some(required)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decision {
    pub candidate_id: Option<u32>,
    pub choice: Choice,
    pub reason: Reason,
    pub stage: StageSummary,
}

/// Positive means a is supported over b. This is a conservative comparison rule,
/// not a hypothesis test of treatment effect or a 95% confidence claim for advice.
pub fn clear_order(a: Rate, b: Rate) -> Option<std::cmp::Ordering> {
    if !valid_rate(a) || !valid_rate(b) {
        return None;
    }
    let (na, nb) = (a.games?, b.games?);
    if na < MIN_COMPARISON_SAMPLES || nb < MIN_COMPARISON_SAMPLES {
        return None;
    }
    let (ia, ib) = (interval95(a.value, na)?, interval95(b.value, nb)?);
    if a.value - b.value + 1e-12 >= MIN_DIFFERENCE && ia.lower > ib.upper {
        Some(Ordering::Greater)
    } else if b.value - a.value + 1e-12 >= MIN_DIFFERENCE && ib.lower > ia.upper {
        Some(Ordering::Less)
    } else {
        None
    }
}

#[derive(Clone, Debug)]
enum Basis {
    Overall,
    Stages,
    Combo(Vec<u32>),
}
#[derive(Clone, Debug)]
struct Edge {
    winner: usize,
    loser: usize,
    basis: Basis,
}

fn valid_rate(rate: Rate) -> bool {
    rate.value.is_finite()
        && (0.0..=1.0).contains(&rate.value)
        && rate.games != Some(0)
        && !rate.wins.is_some_and(|w| rate.games.is_some_and(|n| w > n))
}
fn supported(rate: Rate) -> bool {
    valid_rate(rate) && rate.games.is_some_and(|n| n >= MIN_COMPARISON_SAMPLES)
}
fn source_score(c: &Candidate) -> Option<f64> {
    c.score.filter(|s| s.is_finite() && (0.0..=1.0).contains(s))
}
fn candidate_id(c: &Candidate) -> Option<u32> {
    c.id.filter(|id| *id > 0)
}
fn prerequisite_key(row: &Combination, candidate: Option<u32>) -> Option<Vec<u32>> {
    let mut ids = row.required.clone();
    ids.sort_unstable();
    if ids.is_empty()
        || ids.len() > 3
        || ids.iter().any(|id| *id == 0 || Some(*id) == candidate)
        || ids.windows(2).any(|w| w[0] == w[1])
    {
        None
    } else {
        Some(ids)
    }
}

type CombinationGroups = BTreeMap<Vec<u32>, Vec<Rate>>;

fn combination_groups(c: &Candidate) -> CombinationGroups {
    let mut groups = BTreeMap::<_, Vec<_>>::new();
    for row in &c.current {
        if let Some(key) = prerequisite_key(row, c.id) {
            if valid_rate(row.rate) {
                groups.entry(key).or_default().push(row.rate);
            }
        }
    }
    groups
}

/// Compare only identical confirmed prerequisites. Opposite supported directions
/// conflict; overlapping cohorts are never averaged or counted as extra support.
fn combination_order(
    a: &CombinationGroups,
    b: &CombinationGroups,
) -> (Option<(Ordering, Vec<u32>)>, bool) {
    let mut direction = None;
    let mut chosen: Option<(Vec<u32>, u64)> = None;
    for (key, xs) in a {
        let Some(ys) = b.get(key) else { continue };
        for x in xs {
            for y in ys {
                let Some(order) = clear_order(*x, *y) else {
                    continue;
                };
                if direction.is_some_and(|old| old != order) {
                    return (None, true);
                }
                direction = Some(order);
                let n = x.games.unwrap().min(y.games.unwrap());
                if chosen.as_ref().is_none_or(|(old, samples)| {
                    (key.len(), n) > (old.len(), *samples)
                        || ((key.len(), n) == (old.len(), *samples) && key < old)
                }) {
                    chosen = Some((key.clone(), n));
                }
            }
        }
    }
    (direction.zip(chosen.map(|c| c.0)), false)
}

fn stage_order(a: &Candidate, b: &Candidate) -> (Option<std::cmp::Ordering>, bool) {
    let orders: [Option<Ordering>; 4] =
        std::array::from_fn(|i| clear_order(a.stages[i]?, b.stages[i]?));
    let reversal =
        orders.contains(&Some(Ordering::Greater)) && orders.contains(&Some(Ordering::Less));
    let consistent = orders[0].filter(|o| orders.iter().all(|v| *v == Some(*o)));
    (consistent, reversal)
}

fn source_order(a: &Candidate, b: &Candidate) -> Option<Ordering> {
    let order = source_score(a)?.total_cmp(&source_score(b)?);
    (order != Ordering::Equal).then_some(order)
}

fn edge(i: usize, j: usize, order: Ordering, basis: Basis) -> Edge {
    let (winner, loser) = if order == Ordering::Greater {
        (i, j)
    } else {
        (j, i)
    };
    Edge {
        winner,
        loser,
        basis,
    }
}

fn cycle(edges: &[Edge]) -> bool {
    let mut remaining = vec![0, 1, 2];
    while !remaining.is_empty() {
        let next = remaining.iter().copied().find(|i| {
            !edges
                .iter()
                .any(|e| e.loser == *i && remaining.contains(&e.winner))
        });
        let Some(next) = next else { return true };
        remaining.retain(|i| *i != next);
    }
    false
}

fn specificity(basis: &Basis) -> (u8, usize) {
    match basis {
        Basis::Overall => (0, 0),
        Basis::Stages => (1, 0),
        Basis::Combo(ids) => (2, ids.len()),
    }
}

/// Each pair has an explicit order: confirmed comparable combinations, then all
/// four stages, then source rank. Context replaces only that pair's source edge.
/// A preferred choice must directly beat BOTH alternatives; different prerequisite
/// groups cannot create that evidence transitively. Conflicts/cycles fall back to
/// the unchanged source order, with no negative contextual recommendation.
pub fn decide(cards: &[Candidate; 3]) -> [Decision; 3] {
    let groups: [CombinationGroups; 3] = std::array::from_fn(|i| combination_groups(&cards[i]));
    let mut out: [Decision; 3] = std::array::from_fn(|i| {
        let c = &cards[i];
        let reported = c
            .stages
            .iter()
            .flatten()
            .filter(|r| valid_rate(**r))
            .count() as u8;
        let n = c.stages.iter().flatten().filter(|r| supported(**r)).count() as u8;
        let known = source_score(c).is_some()
            || c.overall.is_some_and(valid_rate)
            || reported > 0
            || !groups[i].is_empty();
        let future = c
            .future
            .iter()
            .any(|r| valid_rate(r.rate) && prerequisite_key(r, c.id).is_some());
        Decision {
            candidate_id: candidate_id(c),
            choice: if candidate_id(c).is_none() {
                Choice::Insufficient
            } else if known {
                Choice::Optional
            } else if future {
                Choice::Conditional
            } else {
                Choice::Insufficient
            },
            reason: if source_score(c).is_some() {
                Reason::OverallRank
            } else if !known && future {
                Reason::FutureOnly
            } else {
                Reason::MissingEvidence
            },
            stage: StageSummary {
                reported,
                supported: n,
                pattern: match (reported, n) {
                    (0, _) => StagePattern::Unavailable,
                    (0..=3, _) => StagePattern::Partial,
                    (_, 0..=3) => StagePattern::LowSample,
                    _ => StagePattern::NoClearOrder,
                },
            },
        }
    });
    let complete = cards.iter().all(|c| candidate_id(c).is_some())
        && (0..3).all(|i| (i + 1..3).all(|j| cards[i].id != cards[j].id));
    let mut edges = Vec::new();
    let mut source_edges = Vec::new();
    let mut conflict = false;
    for i in 0..3 {
        for j in i + 1..3 {
            if candidate_id(&cards[i]).is_none()
                || candidate_id(&cards[j]).is_none()
                || cards[i].id == cards[j].id
            {
                continue;
            }
            let (stage, reversal) = stage_order(&cards[i], &cards[j]);
            for k in [i, j] {
                if reversal {
                    out[k].stage.pattern = StagePattern::Reversal;
                } else if stage.is_some() && out[k].stage.pattern != StagePattern::Reversal {
                    out[k].stage.pattern = StagePattern::Consistent;
                }
            }
            let (combo, contradicted) = combination_order(&groups[i], &groups[j]);
            conflict |= contradicted;
            let source = source_order(&cards[i], &cards[j]);
            if let Some(order) = source {
                source_edges.push(edge(i, j, order, Basis::Overall));
            }
            // Confirmed matching prerequisites are more specific than standalone stages.
            let supported = combo
                .map(|(o, key)| (o, Basis::Combo(key)))
                .or_else(|| stage.map(|o| (o, Basis::Stages)))
                .or_else(|| source.map(|o| (o, Basis::Overall)));
            if let Some((order, basis)) = supported {
                edges.push(edge(i, j, order, basis));
            }
        }
    }
    conflict |= cycle(&edges);
    if conflict {
        edges = source_edges;
    }
    let winner = (0..3).find(|i| {
        complete && (0..3).all(|j| j == *i || edges.iter().any(|e| e.winner == *i && e.loser == j))
    });
    if let Some(winner) = winner {
        let explaining = edges
            .iter()
            .filter(|e| e.winner == winner)
            .max_by(|a, b| {
                specificity(&a.basis)
                    .cmp(&specificity(&b.basis))
                    .then_with(|| cards[b.loser].id.cmp(&cards[a.loser].id))
            })
            .unwrap();
        out[winner].choice = Choice::Preferred;
        out[winner].reason = if conflict {
            Reason::ConflictingEvidence
        } else {
            match &explaining.basis {
                Basis::Overall => Reason::OverallRank,
                Basis::Stages => Reason::StageAdvantage {
                    other: cards[explaining.loser].id.unwrap(),
                },
                Basis::Combo(required) => Reason::ComboAdvantage {
                    other: cards[explaining.loser].id.unwrap(),
                    required: required.clone(),
                },
            }
        };
        for e in edges.iter().filter(|e| e.winner == winner) {
            let reason = match &e.basis {
                Basis::Overall => continue,
                Basis::Stages => Reason::StageAlternative {
                    other: cards[winner].id.unwrap(),
                },
                Basis::Combo(required) => Reason::ComboAlternative {
                    other: cards[winner].id.unwrap(),
                    required: required.clone(),
                },
            };
            out[e.loser].choice = Choice::NotAdvised;
            out[e.loser].reason = reason;
        }
        if conflict {
            for i in 0..3 {
                out[i].reason = Reason::ConflictingEvidence;
            }
        }
    } else {
        // Without three valid source scores, absence of an edge is missing evidence,
        // not an assertion that two reported rates or source ranks are equivalent.
        let source_tie = complete && cards.iter().all(|c| source_score(c).is_some());
        for i in 0..3 {
            if candidate_id(&cards[i]).is_some() && out[i].choice == Choice::Optional {
                out[i].reason = if !complete {
                    Reason::PartialOffer
                } else if conflict {
                    Reason::ConflictingEvidence
                } else if source_tie {
                    Reason::ComparableTie
                } else {
                    Reason::MissingEvidence
                };
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(value: f64, games: u64) -> Rate {
        Rate {
            value,
            games: Some(games),
            wins: None,
        }
    }
    fn row(required: &[u32], value: f64) -> Combination {
        Combination {
            required: required.to_vec(),
            rate: rate(value, 10_000),
        }
    }
    fn cards() -> [Candidate; 3] {
        std::array::from_fn(|i| Candidate {
            id: Some(i as u32 + 1),
            score: Some(0.9 - i as f64 * 0.1),
            overall: Some(rate(0.45 - i as f64 * 0.01, 10_000)),
            ..Default::default()
        })
    }
    fn preferred(decisions: &[Decision; 3]) -> Option<u32> {
        let winners: Vec<_> = decisions
            .iter()
            .filter(|d| d.choice == Choice::Preferred)
            .collect();
        assert!(winners.len() <= 1);
        winners.first().and_then(|d| d.candidate_id)
    }
    fn no_negative(decisions: &[Decision; 3]) {
        assert!(decisions.iter().all(|d| d.choice != Choice::NotAdvised));
    }

    #[test]
    fn rates_below_a_hero_reference_are_not_a_veto_or_a_negative_recommendation() {
        let cards = cards();
        // Hero reference is deliberately not an input to the choice engine.
        assert!(cards.iter().all(|c| c.overall.unwrap().value < 0.55));
        let result = decide(&cards);
        assert_eq!(preferred(&result), Some(1));
        assert_eq!(result[0].reason, Reason::OverallRank);
        no_negative(&result);
    }

    #[test]
    fn complete_supported_stages_can_override_source_order() {
        let mut cards = cards();
        for (c, value) in cards.iter_mut().zip([0.6, 0.8, 0.5]) {
            c.stages = [Some(rate(value, 10_000)); 4];
        }
        let result = decide(&cards);
        assert_eq!(preferred(&result), Some(2));
        assert_eq!(result[1].reason, Reason::StageAdvantage { other: 1 });
        assert_eq!(result[0].choice, Choice::NotAdvised);
        assert_eq!(result[2].choice, Choice::NotAdvised);
        assert!(result
            .iter()
            .all(|d| d.stage.pattern == StagePattern::Consistent));
    }

    #[test]
    fn missing_small_or_overlapping_stage_evidence_cannot_override() {
        let mut full = cards();
        full[0].stages = [Some(rate(0.6, 10_000)); 4];
        full[1].stages = [Some(rate(0.8, 10_000)); 4];
        let mut missing = full.clone();
        missing[1].stages[3] = None;
        let result = decide(&missing);
        assert_eq!(preferred(&result), Some(1));
        assert_eq!(result[1].stage.pattern, StagePattern::Partial);
        no_negative(&result);
        let mut low = full;
        low[1].stages[3] = Some(rate(0.8, 499));
        let result = decide(&low);
        assert_eq!(preferred(&result), Some(1));
        assert_eq!(result[1].stage.pattern, StagePattern::LowSample);
        no_negative(&result);
        let mut overlap = cards();
        overlap[0].stages = [Some(rate(0.50, 500)); 4];
        overlap[1].stages = [Some(rate(0.52, 500)); 4];
        let result = decide(&overlap);
        assert_eq!(preferred(&result), Some(1));
        assert_eq!(result[1].stage.pattern, StagePattern::NoClearOrder);
        no_negative(&result);
    }

    #[test]
    fn stage_reversal_is_visible_without_guessing_a_current_stage() {
        let mut cards = cards();
        cards[0].stages = [0.6, 0.6, 0.8, 0.8].map(|p| Some(rate(p, 10_000)));
        cards[1].stages = [0.8, 0.8, 0.6, 0.6].map(|p| Some(rate(p, 10_000)));
        let result = decide(&cards);
        assert_eq!(preferred(&result), Some(1));
        assert_eq!(result[0].stage.pattern, StagePattern::Reversal);
        assert_eq!(result[1].stage.pattern, StagePattern::Reversal);
        no_negative(&result);
    }

    #[test]
    fn only_current_combinations_with_identical_prerequisites_change_the_choice() {
        let mut cards = cards();
        cards[0].future = vec![row(&[90, 91], 0.5)];
        cards[1].future = vec![row(&[91, 90], 0.8)];
        assert_eq!(preferred(&decide(&cards)), Some(1));
        for c in &mut cards {
            c.current = std::mem::take(&mut c.future);
        }
        let result = decide(&cards);
        assert_eq!(preferred(&result), Some(2));
        assert_eq!(
            result[1].reason,
            Reason::ComboAdvantage {
                other: 1,
                required: vec![90, 91]
            }
        );
        assert_eq!(result[0].choice, Choice::NotAdvised);
        assert_eq!(result[2].choice, Choice::Optional);
        for c in &mut cards {
            c.future = std::mem::take(&mut c.current);
        }
        assert_eq!(preferred(&decide(&cards)), Some(1));
    }

    #[test]
    fn missing_or_different_combo_prerequisites_are_neutral() {
        let mut cards = cards();
        cards[0].current = vec![row(&[90], 0.4)];
        cards[1].current = vec![row(&[91], 0.9)];
        let result = decide(&cards);
        assert_eq!(preferred(&result), Some(1));
        no_negative(&result);
        cards[1].current = vec![Combination {
            required: vec![90],
            rate: rate(0.9, 499),
        }];
        let result = decide(&cards);
        assert_eq!(preferred(&result), Some(1));
        no_negative(&result);
        cards[0].current.clear();
        cards[1].current[0].rate = rate(0.9, 10_000);
        let result = decide(&cards);
        assert_eq!(preferred(&result), Some(1));
        no_negative(&result);
    }

    #[test]
    fn contradictory_prerequisite_groups_are_not_averaged_or_cherry_picked() {
        let mut cards = cards();
        cards[0].current = vec![row(&[90], 0.9), row(&[91], 0.4)];
        cards[1].current = vec![row(&[90], 0.5), row(&[91], 0.8)];
        let result = decide(&cards);
        assert_eq!(preferred(&result), Some(1));
        assert!(result
            .iter()
            .all(|d| d.reason == Reason::ConflictingEvidence));
        no_negative(&result);
    }

    #[test]
    fn confirmed_combo_is_more_specific_than_standalone_stage_history() {
        let mut cards = cards();
        cards[0].stages = [Some(rate(0.9, 10_000)); 4];
        cards[1].stages = [Some(rate(0.6, 10_000)); 4];
        cards[0].current = vec![row(&[90], 0.5)];
        cards[1].current = vec![row(&[90], 0.8)];
        let result = decide(&cards);
        assert_eq!(preferred(&result), Some(2));
        assert_eq!(
            result[1].reason,
            Reason::ComboAdvantage {
                other: 1,
                required: vec![90]
            }
        );
    }

    fn indirect_chain() -> [Candidate; 3] {
        let mut cards = cards();
        cards[0].score = None;
        cards[0].overall = None;
        cards[0].current = vec![row(&[90], 0.8)];
        cards[1].current = vec![row(&[90], 0.5), row(&[91], 0.8)];
        cards[2].current = vec![row(&[91], 0.5)];
        cards
    }

    #[test]
    fn different_prerequisite_paths_cannot_replace_a_missing_direct_comparison() {
        let result = decide(&indirect_chain());
        assert_eq!(preferred(&result), None);
        assert!(result.iter().all(|d| d.reason == Reason::MissingEvidence));
        no_negative(&result);
    }

    #[test]
    fn cycles_containing_both_source_and_context_edges_fall_back_to_source() {
        let mut cards = indirect_chain();
        cards[0].score = Some(0.5); // A > B via X, B > C via Y, C > A via source.
        let result = decide(&cards);
        assert_eq!(preferred(&result), Some(2));
        assert!(result
            .iter()
            .all(|d| d.reason == Reason::ConflictingEvidence));
        no_negative(&result);
    }

    #[test]
    fn missing_scores_are_not_ties_and_rates_do_not_invent_an_overall_rank() {
        let mut cards = cards();
        for (c, p) in cards.iter_mut().zip([0.7, 0.6, 0.5]) {
            c.score = None;
            c.overall = Some(rate(p, 10_000));
        }
        let result = decide(&cards);
        assert_eq!(preferred(&result), None);
        assert!(result
            .iter()
            .all(|d| d.choice == Choice::Optional && d.reason == Reason::MissingEvidence));
        for c in &mut cards {
            c.score = Some(0.5);
        }
        let result = decide(&cards);
        assert_eq!(preferred(&result), None);
        assert!(result.iter().all(|d| d.reason == Reason::ComparableTie));
    }

    #[test]
    fn incomplete_unknown_or_duplicate_offers_never_receive_a_gold_or_negative_choice() {
        for id in [None, Some(0), Some(2)] {
            let mut cards = cards();
            cards[2].id = id;
            cards[0].current = vec![row(&[90], 0.8)];
            cards[1].current = vec![row(&[90], 0.5)];
            let result = decide(&cards);
            assert_eq!(preferred(&result), None);
            no_negative(&result);
        }
        let mut cards = cards();
        cards[2] = Candidate {
            id: Some(3),
            ..Default::default()
        };
        let result = decide(&cards);
        assert_eq!(preferred(&result), None);
        assert_eq!(result[2].choice, Choice::Insufficient);
        no_negative(&result);
    }

    #[test]
    fn future_only_is_conditional_without_a_comparative_or_negative_claim() {
        let cards = std::array::from_fn(|i| Candidate {
            id: Some(i as u32 + 1),
            future: vec![row(&[90], 0.9 - i as f64 * 0.2)],
            ..Default::default()
        });
        let result = decide(&cards);
        assert_eq!(preferred(&result), None);
        assert!(result
            .iter()
            .all(|d| d.choice == Choice::Conditional && d.reason == Reason::FutureOnly));
    }

    #[test]
    fn comparison_thresholds_and_invalid_values_are_explicit() {
        assert_eq!(
            clear_order(rate(0.52, 100_000), rate(0.50, 100_000)),
            Some(Ordering::Greater)
        );
        assert_eq!(
            clear_order(rate(0.50, 100_000), rate(0.52, 100_000)),
            Some(Ordering::Less)
        );
        assert_eq!(
            clear_order(rate(0.519, 1_000_000), rate(0.5, 1_000_000)),
            None
        );
        assert_eq!(clear_order(rate(0.9, 499), rate(0.1, 10_000)), None);
        assert_eq!(
            clear_order(rate(0.9, 500), rate(0.1, 500)),
            Some(Ordering::Greater)
        );
        for invalid in [
            rate(f64::NAN, 10_000),
            rate(f64::INFINITY, 10_000),
            rate(1.1, 10_000),
            rate(-0.1, 10_000),
            rate(0.9, 0),
            Rate {
                value: 0.9,
                games: None,
                wins: None,
            },
            Rate {
                value: 0.9,
                games: Some(10_000),
                wins: Some(10_001),
            },
        ] {
            assert_eq!(clear_order(invalid, rate(0.1, 10_000)), None);
        }
        let mut cards = cards();
        cards[0].score = Some(f64::NAN);
        assert_eq!(preferred(&decide(&cards)), None);
    }

    #[test]
    fn explanation_and_selection_are_stable_under_card_and_combo_row_permutations() {
        let mut original = cards();
        for (c, p) in original.iter_mut().zip([0.5, 0.8, 0.4]) {
            c.current = vec![row(&[91, 90], p), row(&[92], p), row(&[93], p)];
        }
        let by_id = |result: [Decision; 3]| {
            result
                .into_iter()
                .map(|d| (d.candidate_id.unwrap(), d))
                .collect::<BTreeMap<_, _>>()
        };
        let expected = by_id(decide(&original));
        assert_eq!(
            expected[&2].reason,
            Reason::ComboAdvantage {
                other: 1,
                required: vec![90, 91]
            }
        );
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let shuffled = order.map(|i| {
                let mut c = original[i].clone();
                c.current.reverse();
                for row in &mut c.current {
                    row.required.reverse();
                }
                c
            });
            assert_eq!(by_id(decide(&shuffled)), expected);
        }
    }

    #[test]
    fn malformed_prerequisites_never_supply_contextual_edges() {
        for required in [vec![], vec![0], vec![2], vec![90, 90], vec![90, 91, 92, 93]] {
            let mut cards = cards();
            cards[0].current = vec![row(&required, 0.4)];
            cards[1].current = vec![row(&required, 0.9)];
            let result = decide(&cards);
            assert_eq!(preferred(&result), Some(1));
            no_negative(&result);
        }
    }

    #[test]
    fn frozen_aramkit_rows_replay_confirmed_prerequisites_without_predicting_an_offer() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/aramkit-16.22-kaisa-combos.json"
        ))
        .unwrap();
        assert_eq!(fixture["source"]["dataDate"], "2026-11-12");
        assert_eq!(fixture["source"]["subset"], true);
        // Mixed rarities: an evidence replay, not a claim this was a real same-screen offer.
        assert_eq!(fixture["resources"][0]["rarity"], "prismatic");
        assert_eq!(fixture["resources"][1]["rarity"], "gold");
        let total = fixture["source"]["sourceAugmentRows"].as_f64().unwrap();
        let build = |confirmed: bool| {
            std::array::from_fn(|i| {
                let raw = &fixture["overallRows"][i];
                let pair = &fixture["combinationRows"][i];
                let id = raw["id"].as_u64().unwrap() as u32;
                let combination = Combination {
                    required: pair["augmentIds"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_u64().unwrap() as u32)
                        .filter(|other| *other != id)
                        .collect(),
                    rate: rate(
                        pair["winRate"].as_f64().unwrap(),
                        pair["sampleCount"].as_u64().unwrap(),
                    ),
                };
                Candidate {
                    id: Some(id),
                    score: Some((total - raw["rank"].as_f64().unwrap()) / (total - 1.)),
                    overall: Some(rate(
                        raw["winRate"].as_f64().unwrap(),
                        raw["sampleCount"].as_u64().unwrap(),
                    )),
                    current: if confirmed {
                        vec![combination.clone()]
                    } else {
                        vec![]
                    },
                    future: if confirmed { vec![] } else { vec![combination] },
                    ..Default::default()
                }
            })
        };
        let before = decide(&build(false));
        assert_eq!(preferred(&before), Some(1029));
        no_negative(&before);
        let confirmed = build(true);
        assert_eq!(confirmed[0].current[0].rate, rate(0.6505, 26_552));
        assert_eq!(confirmed[2].current[0].rate, rate(0.6276, 42_545));
        assert_eq!(
            clear_order(confirmed[0].current[0].rate, confirmed[1].current[0].rate),
            None
        );
        let after = decide(&confirmed);
        assert_eq!(preferred(&after), Some(1004));
        assert_eq!(
            after[0].reason,
            Reason::ComboAdvantage {
                other: 1029,
                required: vec![3086]
            }
        );
        assert_eq!(after[1].choice, Choice::Optional);
        assert_eq!(after[2].choice, Choice::NotAdvised);
        assert_eq!(decide(&build(false)), before);
    }
}
