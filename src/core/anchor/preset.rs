use super::ReferencePositions;

use std::collections::HashMap;

pub struct AnchorPreset {
    ref_positions: HashMap<usize, Vec<RefPositionsOfPattern>>,
}

impl AnchorPreset {
    pub fn new() -> Self {
        Self {
            ref_positions: HashMap::new(),
        }
    }
    pub fn convert_positions_to_preset(&mut self, pattern_idx: usize, reference_positions: ReferencePositions) {
        for (ref_idx, ref_sorted_positions) in reference_positions {
            let new_ref_position = RefPositionsOfPattern::new(pattern_idx, ref_sorted_positions);
            match self.ref_positions.get_mut(&ref_idx) {
                Some(ref_positions) => {
                    ref_positions.push(new_ref_position);
                },
                None => {
                    self.ref_positions.insert(ref_idx, vec![new_ref_position]);
                },
            }
        }
    }
}

struct RefPositionsOfPattern {
    pattern_idx: usize,
    ref_sorted_positions: Vec<usize>,    
}

impl RefPositionsOfPattern {
    fn new(pattern_idx: usize, ref_sorted_positions: Vec<usize>) -> Self {
        Self {
            pattern_idx,
            ref_sorted_positions,
        }
    }
}