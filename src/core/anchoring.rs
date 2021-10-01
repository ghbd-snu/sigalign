use super::{Cutoff, Penalties, MinPenaltyForPattern};
use super::{Reference, Sequence};
use super::{Anchors, Anchor, Estimation, CheckPoints};

mod preset;

pub use preset::AnchorsPreset;

use std::collections::HashMap;

const PATTERN_INDEX_GAP_FOR_CHECK_POINTS: usize = 3;

impl Anchors {
    pub fn create_preset_by_record(
        reference: &dyn Reference,
        query: Sequence,
        pattern_size: usize,
    ) -> HashMap<usize, AnchorsPreset> {
        AnchorsPreset::new_by_record(reference, query, pattern_size)
    }
    pub fn from_preset(
        anchors_preset: AnchorsPreset,
        record_length: usize,
        query: Sequence,
        pattern_size: usize,
        cutoff: &Cutoff,
        penalties: &Penalties,
        min_penalty_for_pattern: &MinPenaltyForPattern,
    ) -> Self {
        let mut anchors = anchors_preset.to_anchors_for_semi_global(
            pattern_size,
            query.len(),
            record_length,
            min_penalty_for_pattern,
        );
        anchors.create_checkpoints_between_anchors(pattern_size, penalties, cutoff);
        
        anchors
    }
    fn create_checkpoints_between_anchors(
        &mut self,
        pattern_size: usize,
        penalties: &Penalties,
        cutoff: &Cutoff,
    ) {
        let allowed_gap_between_query_position: usize = pattern_size * PATTERN_INDEX_GAP_FOR_CHECK_POINTS;
        for right_first_anchor_index in 1..self.anchors.len() {
            let (left_anchors, right_anchors) = self.anchors.split_at_mut(right_first_anchor_index);

            let left_anchor = left_anchors.last_mut().unwrap();

            left_anchor.create_checkpoint_to_rights(right_anchors, right_first_anchor_index, allowed_gap_between_query_position, penalties, cutoff);
        }
    }
}

impl Anchor {
    fn create_checkpoint_to_rights(
        &mut self,
        right_anchors: &mut [Self],
        right_first_anchor_index: usize,
        allowed_gap_between_query_position: usize,
        penalties: &Penalties,
        cutoff: &Cutoff,
    ) {
        let left_anchor_index = right_first_anchor_index - 1;
        let mut right_anchor_index = left_anchor_index;

        for right_anchor in right_anchors {
            right_anchor_index += 1;

            let query_optional_gap = right_anchor.query_position.checked_sub(self.query_position + self.size);
            let can_be_connected = match query_optional_gap {
                None => {
                    continue
                },
                Some(query_gap) => {
                    if query_gap > allowed_gap_between_query_position {
                        break;
                    }

                    let record_optional_gap = right_anchor.record_position.checked_sub(self.record_position + self.size);
                    match record_optional_gap {
                        None => {
                            continue;
                        },
                        Some(record_gap) => {
                            let max_gap = record_gap.max(query_gap);
                            let min_gap = record_gap.min(query_gap);

                            let gap_count = max_gap - min_gap;

                            let min_penalty = if gap_count == 0 {
                                0
                            } else {
                                penalties.o + gap_count * penalties.e
                            };

                            let penalty = self.left_estimation.penalty + right_anchor.right_estimation.penalty + min_penalty;
                            let length = self.left_estimation.length + self.size + right_anchor.right_estimation.length + max_gap;
                            let penalty_per_length = penalty as f32 / length as f32;

                            (length >= cutoff.minimum_aligned_length) && (penalty_per_length <= cutoff.penalty_per_length)
                        },
                    }
                },
            };

            if can_be_connected {
                self.right_check_points.add_new_checkpoint(right_anchor_index);
                right_anchor.left_check_points.add_new_checkpoint(left_anchor_index);
            }
        }
    }
}

impl Estimation {
    fn new(penalty: usize, length: usize) -> Self {
        Self {
            penalty,
            length,
        }
    }
}

impl CheckPoints {
    fn empty() -> Self {
        Self(Vec::new())
    }
    fn add_new_checkpoint(&mut self, anchor_index: usize) {
        self.0.push(anchor_index);
    }
}

#[cfg(test)]
#[allow(unused)]
mod tests {
    use super::super::*;
    use super::*;

    use crate::reference::TestReference;

    struct TestAlgorithm;

    impl Algorithm for TestAlgorithm {

    }

    #[test]
    fn print_test_anchors_checkpoints() {
        let test_reference = TestReference::new();

        let query = b"GTATCTGCGCCGGTAGAGAGCCATCAGCTGATGTCCCAGACAGATTGCG";

        let kmer = 10;

        let penalties = Penalties {x: 4, o: 6, e: 3};
        let cutoff = Cutoff { minimum_aligned_length: 30, penalty_per_length: 0.5 };
        let min_penalty_for_pattern = MinPenaltyForPattern { odd: 4, even: 3 };

        let anchors_preset_by_record = Anchors::create_preset_by_record(&test_reference, query, kmer);

        for (record_index, anchors_preset) in anchors_preset_by_record {
            let record_sequence = test_reference.sequence_of_record(record_index);
            let record_length = record_sequence.len();

            let anchors = Anchors::from_preset(anchors_preset, record_length, query, kmer, &cutoff, &penalties, &min_penalty_for_pattern);

            println!("# index: {}", record_index);
            println!("{:#?}", anchors);
        }
    }
}
