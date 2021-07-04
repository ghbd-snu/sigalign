//! Alignment by Anchor
use core::panic;
use std::cmp::{min, max};
use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;
use std::slice::Iter;

use super::{AlignmentResult, FmIndex, Operation, EmpKmer, Cutoff, Scores};
use super::dropout_wfa::{WF, ChkpBacktrace, dropout_wf_align, dropout_inherited_wf_align, wf_backtrace, ChkpInherit, wf_check_inheritable, wf_inherited_cache};
use fm_index::BackwardSearchIndex;

/// Anchor Group
pub struct AnchorGroup<'a> {
    ref_seq: &'a [u8],
    qry_seq: &'a [u8],
    scores: &'a Scores,
    cutoff: &'a Cutoff,
    anchors: Vec<Anchor>,
}
impl<'a> AnchorGroup<'a> {
    pub fn new(
        ref_seq: &'a [u8], qry_seq: &'a [u8], index: &FmIndex,
        kmer: usize, emp_kmer: &'a EmpKmer, scores: &'a Scores, cutoff: &'a Cutoff
    ) -> Option<Self> {
        let ref_len = ref_seq.len();
        let qry_len = qry_seq.len();
        let search_count = qry_len / kmer;
        let mut anchors_preset: Vec<Anchor> = Vec::new();
        let mut anchor_existence: Vec<bool> = Vec::with_capacity(search_count+1); // first value is buffer
        // (1) Generate Anchors Proto
        {
            let mut anchors_cache: Option<Vec<Anchor>> = None;
            for i in 0..search_count {
                let qry_position = i*kmer;
                let pattern = &qry_seq[qry_position..qry_position+kmer];
                let search = index.search_backward(pattern);
                let positions = search.locate();
                // ** Check Impeccable Extension **
                match anchors_cache {
                    Some(anchors) => {
                        if positions.len() == 0 {
                            anchors_preset.extend(anchors);
                            anchors_cache = None;
                        } else {
                            let mut current_anchors: Vec<Anchor> = Vec::with_capacity(positions.len());
                            let mut ie_positions: Vec<u64> = Vec::new();
                            for anchor in anchors {
                                let mut ie_check = false;
                                for position in &positions {
                                    // impeccable extension occurs
                                    if *position as usize == anchor.position.0 + anchor.size {
                                        ie_positions.push(*position);
                                        ie_check = true;
                                        break;
                                    }
                                }
                                // push anchor
                                if !ie_check {
                                    anchors_preset.push(anchor);
                                } else {
                                    current_anchors.push(anchor.impeccable_extension(kmer));
                                }
                            }
                            // if position is not ie position: add to current anchors
                            for position in positions {
                                if !ie_positions.contains(&position) {
                                    current_anchors.push(
                                        Anchor::new(position as usize, i*kmer, kmer)
                                    );
                                }
                            }
                            anchors_cache = Some(current_anchors);
                        }
                        anchor_existence.push(true);
                    },
                    None => {
                        if positions.len() != 0 {
                            anchors_cache = Some(positions.into_iter().map(|x| {
                                Anchor::new(x as usize, i*kmer, kmer)
                            }).collect());
                        }
                        anchor_existence.push(false);
                    },
                }
            }
            // push last anchors
            match anchors_cache {
                Some(anchors) => {
                    anchors_preset.extend(anchors);
                    anchor_existence.push(true);
                },
                None => {
                    anchor_existence.push(false);
                },
            }
        }
        // check anchor exist
        if !anchor_existence.iter().any(|x| *x) {
            return None
        }
        // (2) Calculate the EMP values
        anchors_preset.iter_mut().for_each(|anchor| {
            anchor.estimate_from_empty(ref_len, qry_len, kmer, &anchor_existence, &emp_kmer);
        });
        // (3) evaluate raw anchors
        anchors_preset.iter_mut().for_each(|anchor| {
            if !anchor.is_emp_state_valid(cutoff) {
                anchor.to_dropped();
            }
        });
        // (4) Set up checkpoints
        Anchor::create_check_points(&mut anchors_preset, scores, cutoff);
        Some(
            Self {
                ref_seq: ref_seq,
                qry_seq: qry_seq,
                scores: scores,
                cutoff: cutoff,
                anchors: anchors_preset,
            }
        )
    }
    pub fn alignment(&mut self, using_cached_wf: bool) {
        // (1) alignment hind
        for idx in 0..self.anchors.len() {
            Anchor::alignment(
                &mut self.anchors, idx,
                self.ref_seq, self.qry_seq, self.scores, self.cutoff,
                BlockType::Hind,
                using_cached_wf
            );
        }
        // (2) alignment fore
        let reversed_ref_seq: Vec<u8> = self.ref_seq.iter().rev().map(|x| *x).collect();
        let reversed_qry_seq: Vec<u8> = self.qry_seq.iter().rev().map(|x| *x).collect();
        for idx in (0..self.anchors.len()).rev() {
            Anchor::alignment(
                &mut self.anchors, idx,
                &reversed_ref_seq, &reversed_qry_seq, self.scores, self.cutoff,
                BlockType::Fore,
                using_cached_wf
            );
        };
    }
    pub fn get_result(&mut self, get_minimum_penalty: bool) -> AlignmentResult {
        // (3) evaluate
        let anchors_of_minimum_penalty = if get_minimum_penalty {
            // TODO: first anchor can be evalauted only one time?
            let (mut minimum_penalty, _) = self.anchors[0].get_penalty_and_length();
            let mut anchors_of_minimum_penalty: HashSet<usize> = HashSet::new();
            for (anchor_index, anchor) in self.anchors.iter_mut().enumerate() {
                let (penalty, length) = anchor.get_penalty_and_length();
                if !Anchor::evaluate_exact_alignment(penalty, length, &self.cutoff) {
                    anchor.to_dropped();
                } else {
                    if penalty < minimum_penalty {
                        minimum_penalty = penalty;
                        anchors_of_minimum_penalty = HashSet::from_iter(vec![anchor_index]);
                    } else if penalty == minimum_penalty {
                        anchors_of_minimum_penalty.insert(anchor_index);
                    }
                }
            }
            Some(anchors_of_minimum_penalty)
        } else {
            for anchor in self.anchors.iter_mut() {
                let (penalty, length) = anchor.get_penalty_and_length();
                if !Anchor::evaluate_exact_alignment(penalty, length, &self.cutoff) {
                    anchor.to_dropped();
                };
            };
            None
        };
        // (4) get unique anchors
        let unqiue_anchors_index = Anchor::get_unique_symbols(&self.anchors, anchors_of_minimum_penalty);
        // (5) get operations & penalty
        let ref_len = self.ref_seq.len();
        let qry_len = self.qry_seq.len();
        unqiue_anchors_index.into_iter().map(|anchor_index| {
            Anchor::operations_and_penalty(&self.anchors, anchor_index, ref_len, qry_len)
        }).collect()
    }
}

/// Anchor
#[derive(Debug)]
pub struct Anchor {
    /// Positions of anchor
    /// (position of reference, position of qry)
    position: (usize, usize),
    /// Size of anchor
    size: usize,
    /// Alignment state of anchor
    state: AlignmentState,
    /// Index of other anchors to check on WF inheritance & backtrace.
    /// (fore, hind)
    check_points: (Vec<usize>, Vec<usize>),
    /// Cache for inherited WF
    wf_cache: Option<WF>,
    /// Connected anchors index set for used as anchor's symbol
    connected: HashSet<usize>,
}

/// State of alignment
#[derive(Debug)]
pub enum AlignmentState {
    /// 1st state
    /// fore and hind alignments are empty
    Empty,
    /// 2nd state
    /// filled with blocks in the EMP state
    Estimated(EmpBlock, EmpBlock), // Fore, Hind
    /// 3rd, 4th state
    /// aligned exactly with `dropout wfa`
    Exact(Option<AlignmentBlock>, AlignmentBlock), // Fore, Hind
    /// Cutoff is not satisfied when aligned from anchor
    Dropped,
}
impl AlignmentState {
    fn is_valid(&self) -> bool {
        match self {
            Self::Exact(_, _) => true,
            _ => false,
        }
    }
}

/// Alignment assumed when EMP state from anchor
#[derive(Debug)]
pub struct EmpBlock {
    penalty: usize,
    length: usize,
}
impl EmpBlock {
    fn new(penalty: usize, length: usize) -> Self {
        Self {
            penalty,
            length,
        }
    }
}

/// One-way semi-global alignment from anchor
#[derive(Debug)]
pub enum AlignmentBlock {
    /// Having an operations.
    /// (operations, penalty)
    Own(Vec<Operation>, usize), 
    /// Referring to the operation of another anchor.
    /// (index of connected anchor, reverse index of operation(same as length), penalty)
    Ref(usize, usize, usize),
}
impl AlignmentBlock {
    fn aligned_length(operations: &Iter<Operation>) -> (usize, usize) {
        let ins = operations.clone().filter(|&op| *op == Operation::Ins).count();
        let del = operations.clone().filter(|&op| *op == Operation::Del).count();
        let len = operations.len();
        (len-del, len-ins)
    }
    fn clip_operation(operations: &Iter<Operation>, ref_len: usize, qry_len: usize) -> Operation {
        let (ref_aligned_length, qry_aligned_length) = Self::aligned_length(operations);
        let ref_left = ref_len-ref_aligned_length;
        let qry_left = qry_len-qry_aligned_length;
        if ref_left >= qry_left {
            Operation::RefClip(ref_left-qry_left)
        } else {
            Operation::QryClip(qry_left-ref_left)
        }
    }
}

impl Anchor {
    /**
    # initialization
    */
    /// New anchor in Empty state
    fn new(ref_pos: usize, qry_pos: usize, kmer: usize) -> Self {
        Self {
            position: (ref_pos, qry_pos),
            size: kmer,
            state:AlignmentState::Empty,
            check_points: (Vec::new(), Vec::new()),
            wf_cache: None,
            connected: HashSet::new(),
        }
    }
    /// When the anchor is completely connected, both anchors are treated as one anchor.
    fn impeccable_extension(mut self, kmer: usize) -> Self {
        self.size += kmer;
        self
    }
    /// Empty anchor to estimated state
    fn estimate_from_empty(&mut self, ref_len: usize, qry_len: usize, kmer: usize, anchor_existence: &Vec<bool>, emp_kmer: &EmpKmer) {
        let block_index = self.position.1 / kmer;
        // fore block
        let fore_emp_block = {
            let block_len = min(self.position.0, self.position.1);
            let quot = block_len / kmer;
            let mut odd_block_count: usize = 0;
            let mut even_block_count: usize = 0;
            let mut previous_block_is_odd = false;
            anchor_existence[(block_index-quot+1)..block_index+1].iter().rev().for_each(|exist| {
                if !*exist {
                    if previous_block_is_odd {
                        even_block_count += 1;
                        previous_block_is_odd = false;
                    } else {
                        odd_block_count += 1;
                        previous_block_is_odd = true;
                    }
                } else {
                    previous_block_is_odd = false;
                }
            });
            EmpBlock::new(
                odd_block_count*emp_kmer.odd + even_block_count*emp_kmer.even,
                block_len + odd_block_count + even_block_count
            )
        };
        // hind block
        let hind_emp_block = {
            let hind_block_index = block_index+(self.size/kmer);
            let ref_block_len = ref_len - (self.position.0 + self.size);
            let qry_block_len = qry_len - (self.position.1 + self.size);
            let block_len = min(ref_block_len, qry_block_len);
            let quot = block_len / kmer;
            let mut odd_block_count: usize = 0;
            let mut even_block_count: usize = 0;
            let mut previous_block_is_odd = false;
            anchor_existence[hind_block_index+1..hind_block_index+quot+1].iter().for_each(|exist| {
                if !*exist {
                    if previous_block_is_odd {
                        even_block_count += 1;
                        previous_block_is_odd = false;
                    } else {
                        odd_block_count += 1;
                        previous_block_is_odd = true;
                    }
                } else {
                    previous_block_is_odd = false;
                }
            });
            EmpBlock::new(
                odd_block_count*emp_kmer.odd + even_block_count*emp_kmer.even,
                block_len + odd_block_count + even_block_count
            )
        };
        self.state = AlignmentState::Estimated(fore_emp_block, hind_emp_block);
    }
    fn is_emp_state_valid(&self, cutoff: &Cutoff) -> bool{
        if let AlignmentState::Estimated(emp_block_1, emp_block_2) = &self.state {
            let length = emp_block_1.length + emp_block_2.length + self.size;
            if length >= cutoff.minimum_length && (emp_block_1.penalty + emp_block_2.penalty) as f64/length as f64 <= cutoff.score_per_length {
                true
            } else {
                false
            }
        } else {
            panic!("Anchor is not in EMP state.");
        }
    }
    /**
    Check point
    */
    // query block stacked in order in anchors_preset
    // : high index is always the hind anchor
    fn can_be_connected(first: &Self, second: &Self, scores: &Scores, cutoff: &Cutoff) -> bool {
        let ref_gap = second.position.0 as i64 - first.position.0 as i64 - first.size as i64;
        let qry_gap = second.position.1 as i64 - first.position.1 as i64 - first.size as i64;
        if (ref_gap >= 0) && (qry_gap >= 0) {
            let mut penalty: usize = 0;
            let mut length: usize = 0;
            // fore
            if let AlignmentState::Estimated(emp_block, _) = &first.state {
                penalty += emp_block.penalty;
                length += emp_block.length;
            }
            // hind
            if let AlignmentState::Estimated(_, emp_block) = &second.state {
                penalty += emp_block.penalty;
                length += emp_block.length;
            }
            // middle
            length += max(ref_gap, qry_gap) as usize + first.size + second.size;
            let indel = (ref_gap - qry_gap).abs() as usize;
            if indel > 0 {
                penalty += scores.1 + indel*scores.2;
            }
            if (penalty as f64 / length as f64 <= cutoff.score_per_length) & (length >= cutoff.minimum_length) {
                true
            } else {
                false
            }
        } else {
            false
        }
    }
    fn extend_each_check_points(anchors: &mut Vec<Self>, first_index: usize, second_index: usize) {
        anchors[first_index].check_points.1.push(second_index);
        anchors[second_index].check_points.0.push(first_index);
    }
    fn both_estimated(anchor_1: &Self, anchor_2: &Self) -> bool {
        (match &anchor_1.state {
            AlignmentState::Estimated(_, _) => true,
            _ => false,
        }) && (match &anchor_2.state {
            AlignmentState::Estimated(_, _) => true,
            _ => false,
        })
    }
    fn create_check_points(anchors: &mut Vec<Self>, scores: &Scores, cutoff: &Cutoff) {
        let anchor_count = anchors.len();
        for index_1 in 0..anchor_count {
            for index_2 in index_1+1..anchor_count {
                if Self::both_estimated(&anchors[index_1], &anchors[index_2]) && Self::can_be_connected(&anchors[index_1], &anchors[index_2], &scores, &cutoff) {
                    Self::extend_each_check_points(anchors, index_1, index_2);
                }
            }
        }
    }
    fn wf_backtrace_check_points(anchors: &Vec<Self>, current_index: usize, block_type: BlockType) -> ChkpBacktrace {
        let current_anchor = &anchors[current_index];
        match block_type {
            BlockType::Fore => {
                let check_points = &current_anchor.check_points.0;
                let mut backtrace_check_points: ChkpBacktrace = Vec::with_capacity(check_points.len());
                check_points.into_iter().for_each(|&anchor_index| {
                    let anchor = &anchors[anchor_index];
                    let ref_gap = (current_anchor.position.0 - anchor.position.0) as i32;
                    let qry_gap = (current_anchor.position.1 - anchor.position.1) as i32;
                    backtrace_check_points.push((anchor_index, anchor.size as i32, ref_gap-qry_gap, ref_gap));
                });
                backtrace_check_points
            },
            BlockType::Hind => {
                let check_points = &current_anchor.check_points.1;
                let mut backtrace_check_points: ChkpBacktrace = Vec::with_capacity(check_points.len());
                check_points.into_iter().for_each(|&anchor_index| {
                    let anchor = &anchors[anchor_index];
                    let ref_gap = (anchor.position.0 + anchor.size - current_anchor.position.0 - current_anchor.size) as i32;
                    let qry_gap = (anchor.position.1 + anchor.size - current_anchor.position.1 - current_anchor.size) as i32;
                    backtrace_check_points.push((anchor_index, anchor.size as i32, ref_gap-qry_gap, ref_gap));
                });
                backtrace_check_points
            },
        }
    }
    fn wf_inheritance_check_points(anchors: &Vec<Self>, current_index: usize, ref_seq: &[u8], qry_seq: &[u8], block_type: BlockType) -> ChkpInherit{
        let current_anchor = &anchors[current_index];
        match block_type {
            BlockType::Fore => {
                let check_points = &current_anchor.check_points.0;
                let mut inheritance_check_points: ChkpInherit = HashMap::with_capacity(check_points.len());
                check_points.iter().for_each(|&anchor_index| {
                    let anchor = &anchors[anchor_index];
                    if let AlignmentState::Exact(None, _) = &anchor.state {
                        let (ref_pos, qry_pos) = anchor.position;
                        let mut ext_count: usize = 1;
                        loop {
                            if let Some(ref_char) = ref_seq.get(ref_pos - ext_count) {
                                if let Some(qry_char) = qry_seq.get(qry_pos - ext_count) {
                                    if *ref_char == *qry_char {
                                        ext_count += 1
                                    } else {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        };
                        let ref_gap = (current_anchor.position.0 - anchor.position.0) as i32;
                        let qry_gap = (current_anchor.position.1 - anchor.position.1) as i32;
                        inheritance_check_points.insert(anchor_index, (anchor.size, ref_gap-qry_gap, ref_gap, ref_gap+ext_count as i32-1));
                    };
                });
                inheritance_check_points
            },
            BlockType::Hind => {
                let check_points = &current_anchor.check_points.1;
                let mut inheritance_check_points: ChkpInherit = HashMap::with_capacity(check_points.len());
                check_points.iter().for_each(|&anchor_index| {
                    let anchor = &anchors[anchor_index];
                    if let AlignmentState::Estimated(_, _) = &anchor.state {
                        let (ref_pos, qry_pos) = anchor.position;
                        let mut ext_count: usize = 1;
                        loop {
                            if let Some(ref_char) = ref_seq.get(ref_pos + anchor.size + ext_count) {
                                if let Some(qry_char) = qry_seq.get(qry_pos + anchor.size +  ext_count) {
                                    if *ref_char == *qry_char {
                                        ext_count += 1
                                    } else {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        };
                        let ref_gap = (anchor.position.0 + anchor.size - current_anchor.position.0 - current_anchor.size) as i32;
                        let qry_gap = (anchor.position.1 + anchor.size - current_anchor.position.1 - current_anchor.size) as i32;
                        inheritance_check_points.insert(anchor_index, (anchor.size, ref_gap-qry_gap, ref_gap, ref_gap+ext_count as i32-1));
                    };
                });
                inheritance_check_points
            },
        }
    }
    fn wf_inheritance_check_points_dep(anchors: &Vec<Self>, current_index: usize, block_type: BlockType) -> ChkpBacktrace {
        let current_anchor = &anchors[current_index];
        match block_type {
            BlockType::Fore => {
                let check_points = &current_anchor.check_points.0;
                let mut inheritance_check_points: ChkpBacktrace = Vec::with_capacity(check_points.len());
                check_points.iter().for_each(|&anchor_index| {
                    let anchor = &anchors[anchor_index];
                    if let AlignmentState::Exact(None, _) = &anchor.state {
                        let ref_gap = (current_anchor.position.0 - anchor.position.0) as i32;
                        let qry_gap = (current_anchor.position.1 - anchor.position.1) as i32;
                        inheritance_check_points.push((anchor_index, anchor.size as i32, ref_gap-qry_gap, ref_gap));
                    };
                });
                inheritance_check_points
            },
            BlockType::Hind => {
                let check_points = &current_anchor.check_points.1;
                let mut inheritance_check_points: ChkpBacktrace = Vec::with_capacity(check_points.len());
                check_points.into_iter().for_each(|&anchor_index| {
                    let anchor = &anchors[anchor_index];
                    if let AlignmentState::Estimated(_, _) = &anchor.state {
                        let ref_gap = (anchor.position.0 + anchor.size - current_anchor.position.0 - current_anchor.size) as i32;
                        let qry_gap = (anchor.position.1 + anchor.size - current_anchor.position.1 - current_anchor.size) as i32;
                        inheritance_check_points.push((anchor_index, anchor.size as i32, ref_gap-qry_gap, ref_gap));
                    };
                });
                inheritance_check_points
            },
        }
    }
    /**
    Alignment
    */
    fn alignment(anchors: &mut Vec<Self>, current_anchor_index: usize, ref_seq: &[u8], qry_seq: &[u8], scores: &Scores, cutoff: &Cutoff, block_type: BlockType, using_cached_wf: bool) {
        #[cfg(test)]
        {
            println!("current index: {:?} / pos: {:?}", current_anchor_index, anchors[current_anchor_index].position);
        }
        /**
        (1) get alignment result
        */
        let alignment_res = {
            // get refernce of current anchor
            let current_anchor = &mut anchors[current_anchor_index];
            let (p_other, l_other) = match block_type {
                BlockType::Hind => {
                    match &current_anchor.state {
                        AlignmentState::Estimated(emp_block, _) => {
                            (emp_block.penalty, emp_block.length)
                        },
                        // if not estimated(hind block is already done) or dropped
                        // -> just pass to next
                        _ => return,
                    }
                },
                BlockType::Fore => {
                    match &current_anchor.state {
                        AlignmentState::Exact(fore, hind) => {
                            match fore {
                                None => {
                                    match hind {
                                        AlignmentBlock::Own(operations, penalty) => {
                                            (*penalty, operations.len())
                                        },
                                        AlignmentBlock::Ref(_, reverse_index, penalty ) => {
                                            (*penalty, *reverse_index)
                                        }
                                    }
                                },
                                _ => return,
                            }
                        },
                        _ => return,
                    }
                },
            };
            // if current anchor has cached wf -> continue with cached wf
            let wf_cache = current_anchor.wf_cache.take();
            #[cfg(test)]
            {
                if let Some(v) = &wf_cache {
                    println!("using inherited wf:\n{:?}\n{:?}, ", v[0], v[1]);
                };
            }
            let panalty_spare = match block_type {
                BlockType::Hind => {
                    cutoff.score_per_length * (
                        min(
                            ref_seq.len() - current_anchor.position.0 - current_anchor.size, qry_seq.len() - current_anchor.position.1 - current_anchor.size
                        ) + current_anchor.size + l_other
                    ) as f64 - p_other as f64
                },
                BlockType::Fore => {
                    cutoff.score_per_length * (
                        min(current_anchor.position.0, current_anchor.position.1) + current_anchor.size + l_other
                    ) as f64 - p_other as f64
                }
            };
            match block_type {
                BlockType::Hind => {
                    match wf_cache {
                        Some(wf) => {
                            dropout_inherited_wf_align(wf, &qry_seq[current_anchor.position.1+current_anchor.size..], &ref_seq[current_anchor.position.0+current_anchor.size..], scores, panalty_spare, cutoff.score_per_length)
                        },
                        None => {
                            dropout_wf_align(&qry_seq[current_anchor.position.1+current_anchor.size..], &ref_seq[current_anchor.position.0+current_anchor.size..], scores, panalty_spare, cutoff.score_per_length)
                        },
                    }
                },
                BlockType::Fore => {
                    // sequence must be reversed !
                    match wf_cache {
                        Some(wf) => {
                            dropout_inherited_wf_align(wf, &qry_seq[qry_seq.len()-current_anchor.position.1..], &ref_seq[ref_seq.len()-current_anchor.position.0..], scores, panalty_spare, cutoff.score_per_length)
                        },
                        None => {
                            dropout_wf_align(&qry_seq[qry_seq.len()-current_anchor.position.1..], &ref_seq[ref_seq.len()-current_anchor.position.0..], scores, panalty_spare, cutoff.score_per_length)
                        },
                    }
                },
            }
        };
        /**
        (2) Interpreting result
        */
        match alignment_res {
            /*
            CASE 1: wf not dropped
            */
            Ok((mut wf, last_k)) => {
                // wf inheritant check
                let check_points_values = Self::wf_backtrace_check_points(anchors, current_anchor_index, block_type.clone());
                let (mut operations, connected_backtraces) = wf_backtrace(
                    &mut wf, scores, last_k, &check_points_values
                );
                // if fore block is aligned, reverse the operations
                if let BlockType::Fore = block_type {
                    operations.reverse();
                };
                // get valid anchor index
                let valid_anchors_index: HashSet<usize> = HashSet::from_iter(
                    connected_backtraces.keys().map(|x| *x)
                );
                // update current anchor
                {
                    let current_anchor = &mut anchors[current_anchor_index];
                    // update state
                    match block_type {
                        BlockType::Hind => {
                            current_anchor.state = AlignmentState::Exact(
                                None,
                                AlignmentBlock::Own(operations, wf.len() - 1),
                            );
                        },
                        BlockType::Fore => {
                            if let AlignmentState::Exact(fore_block, _) = &mut current_anchor.state {
                                *fore_block = Some(AlignmentBlock::Own(operations, wf.len() - 1));
                            }
                        }
                    }
                    // update connected anchors
                    current_anchor.connected.extend(valid_anchors_index.iter());
                }
                // update connected anchors
                let current_anchor_score = wf.len() - 1;
                for (anchor_index, (reverse_index, penalty)) in connected_backtraces {
                    let anchor = &mut anchors[anchor_index];
                    // update anchor state & anchor's connected info
                    match block_type {
                        BlockType::Hind => {
                            anchor.state = AlignmentState::Exact(
                                None,
                                AlignmentBlock::Ref(
                                    current_anchor_index,
                                    reverse_index,
                                    current_anchor_score - penalty
                                ),
                            );
                            for check_point in &anchor.check_points.1 {
                                if valid_anchors_index.contains(check_point) {
                                    anchor.connected.insert(*check_point);
                                }
                            }
                        },
                        BlockType::Fore => {
                            if let AlignmentState::Exact(fore_block, _) = &mut anchor.state {
                                *fore_block = Some(AlignmentBlock::Ref(
                                    current_anchor_index,
                                    reverse_index,
                                    current_anchor_score - penalty
                                ));
                            };
                            for check_point in &anchor.check_points.0 {
                                if valid_anchors_index.contains(check_point) {
                                    anchor.connected.insert(*check_point);
                                }
                            };
                        }
                    }
                    // if anchor has cached wf: drop it
                    anchor.wf_cache = None;
                }
            },
            /*
            CASE 2: wf dropped
            */
            // TODO:
            Err(wf) => {
                if using_cached_wf {
                    let check_points_values = Self::wf_inheritance_check_points(anchors, current_anchor_index, ref_seq, qry_seq, block_type.clone());
                    // unpack map & sort by anchor index
                    let inheritable_checkpoints: Vec<(usize, usize, i32, i32, i32)> = {
                        let mut valid_checkpoints: Vec<(usize, usize, i32, i32, i32)> = wf_check_inheritable(&wf, scores, check_points_values).into_iter().map(
                            |(key, val)| {
                                (key, val.0, val.1, val.2, val.3)
                            }
                        ).collect();
                        valid_checkpoints.sort_by(|a, b| a.cmp(&b));
                        valid_checkpoints
                    };
                    let mut checked_anchors_index: HashSet<usize> = HashSet::new();
                    for (anchor_index, score, k, fr, ext_fr) in inheritable_checkpoints {
                        // if anchor is not checked yet: caching WF
                        if !checked_anchors_index.contains(&anchor_index) {
                            let anchor = &mut anchors[anchor_index];
                            // inherit WF
                            anchor.wf_cache = Some(wf_inherited_cache(&wf, score, k, fr, ext_fr));
                            // add all check points to the checked index list
                            checked_anchors_index.insert(anchor_index);
                            match block_type {
                                BlockType::Hind => {
                                    checked_anchors_index.extend(anchor.check_points.1.iter());
                                },
                                BlockType::Fore => {
                                    checked_anchors_index.extend(anchor.check_points.0.iter());
                                },
                            }
                        }
                    }
                }
                // drop current index
                anchors[current_anchor_index].to_dropped();
            },
        }
    }
    fn to_dropped(&mut self) {
        self.state = AlignmentState::Dropped;
    }
    /**
    Evaluate
    */
    fn get_penalty_and_length(&self) -> (usize, usize) {
        let mut total_length: usize = 0;
        let mut total_penalty: usize = 0;
        if let AlignmentState::Exact(fore_option, hind) = &self.state {
            let fore = fore_option.as_ref().unwrap();
            // add fore & hind info
            for &block in [fore, hind].iter() {
                match block {
                    AlignmentBlock::Own(operations, penalty) => {
                        total_length += operations.len();
                        total_penalty += *penalty;
                    },
                    AlignmentBlock::Ref(_, reverse_index, penalty) => {
                        total_length += reverse_index;
                        total_penalty += *penalty;
                    },
                }
            }
        }
        total_length += self.size;
        (total_penalty, total_length)
    }
    fn evaluate_exact_alignment(penalty: usize, length: usize, cutoff: &Cutoff) -> bool {
        if (length >= cutoff.minimum_length) && (penalty as f64/length as f64 <= cutoff.score_per_length) {
            true
        } else {
            false
        }
    }
    fn get_unique_symbols(anchors: &Vec<Self>, anchors_of_minimum_penalty: Option<HashSet<usize>>) -> HashSet<usize> {
        // TODO: can be more optimized
        // valid anchors set
        let valid_anchors_set: HashSet<usize> = match anchors_of_minimum_penalty {
            Some(anchors_set) => anchors_set,
            None => {
                anchors.iter().enumerate().filter_map(
                    |(idx, anchor)| {
                        match anchor.state {
                            AlignmentState::Exact(_, _) => {
                                Some(idx)
                            },
                            _ => {
                                None
                            }
                        }
                    }
                ).collect()
            }
        };
        // symbol dictionary
        let anchor_symbols = {
            let mut anchor_symbols: HashMap<usize, HashSet<usize>> = HashMap::with_capacity(valid_anchors_set.len());
            // 1. add connected & valid anchor
            for &anchor_index in valid_anchors_set.iter() {
                let symbol: HashSet<usize> =  valid_anchors_set.intersection(&anchors[anchor_index].connected).map(|x| *x).collect();
                anchor_symbols.insert(anchor_index, symbol);
            };
            // 2. add extended anchors of connected
            for anchor_index in valid_anchors_set.iter() {
                let mut extended_symbol: HashSet<usize> = HashSet::new();
                anchor_symbols.get(anchor_index).unwrap().iter().for_each(|idx| {
                    extended_symbol.extend(anchor_symbols.get(idx).unwrap());
                });
                let symbol = anchor_symbols.get_mut(anchor_index).unwrap();
                symbol.extend(extended_symbol);
                // add self index
                symbol.insert(*anchor_index);
            };
            anchor_symbols
        };
        // unique symbols list
        let unique_anchor = {
            let mut unique_anchor: HashSet<usize> = HashSet::new();
            let mut used_symbols: HashSet<Vec<usize>> = HashSet::with_capacity(anchor_symbols.len());
            for (anchor_index, symbol) in anchor_symbols.into_iter() {
                let mut serialized_symbol: Vec<usize> = symbol.into_iter().collect();
                serialized_symbol.sort();
                if !used_symbols.contains(&serialized_symbol) {
                    unique_anchor.insert(anchor_index);
                    used_symbols.insert(serialized_symbol);
                }
            };
            unique_anchor
        };
        unique_anchor
    }
    fn operations_and_penalty(anchors: &Vec<Self>, current_anchor_index: usize, ref_len: usize, qry_len: usize) -> (Vec<Operation>, usize) {
        let current_anchor = &anchors[current_anchor_index];
        let mut penalty_result: usize = 0;
        let operations_result = if let AlignmentState::Exact(fore_option, hind) = &current_anchor.state {
            // fore
            let fore = fore_option.as_ref().unwrap();
            let fore_ops_iter = match fore {
                AlignmentBlock::Own(operations, penalty) => {
                    penalty_result += penalty;
                    operations.iter()
                },
                AlignmentBlock::Ref(anchor_index, reverse_index, penalty) => {
                    let anchor = &anchors[*anchor_index];
                    if let AlignmentState::Exact(Some(AlignmentBlock::Own(operations, _)), _) = &anchor.state {
                        penalty_result += penalty;
                        operations[..*reverse_index].iter()
                    } else {
                        // TODO: err msg
                        panic!("Trying to get result operations from invalid anchor.");
                    }
                }
            };
            // hind operations
            let hind_ops_iter = match hind {
                AlignmentBlock::Own(operations, penalty) => {
                    penalty_result += penalty;
                    operations.iter()
                },
                AlignmentBlock::Ref(anchor_index, reverse_index, penalty) => {
                    let anchor = &anchors[*anchor_index];
                    if let AlignmentState::Exact(_, AlignmentBlock::Own(operations, _)) = &anchor.state {
                        penalty_result += penalty;
                        operations[operations.len()-*reverse_index..].iter()
                    } else {
                        // TODO: err msg
                        panic!("Trying to get result operations from invalid anchor.");
                    }
                }
            };
            let mut operations_result: Vec<Operation> = Vec::with_capacity(
                fore_ops_iter.len() + hind_ops_iter.len() + 2
            );
            let fore_clip_operation = AlignmentBlock::clip_operation(&fore_ops_iter, current_anchor.position.0, current_anchor.position.1);
            let hind_clip_operation = AlignmentBlock::clip_operation(&hind_ops_iter, ref_len-current_anchor.position.0-current_anchor.size, qry_len-current_anchor.position.1-current_anchor.size);
            // clip operation of fore
            operations_result.push(fore_clip_operation);
            // operations of fore
            operations_result.extend(fore_ops_iter);
            // operations of kmer block
            operations_result.extend(vec![Operation::Match; current_anchor.size]);
            // operations of hind
            operations_result.extend(hind_ops_iter);
            // clip operation of hind
            operations_result.push(hind_clip_operation);
            operations_result
        } else {
            panic!("Trying to get result operations from invalid anchor.");
        };
        (operations_result, penalty_result)
    }
}

#[derive(Clone)]
enum BlockType {
    Hind,
    Fore,
}

// #[cfg(test)]
// mod tests {
//     use crate::alignment::test_data;
//     use super::*;

//     fn print_anchor_group() {
//         let test_data = test_data::get_test_data();
//         let seqs = test_data[1].clone();
//         let ref_seq = seqs.0.as_bytes();
//         let qry_seq = seqs.1.as_bytes();
//         let index = super::super::Reference::fmindex(&ref_seq);
//         let aligner = super::super::tests::test_aligner(
//             0.05, 100, 3, 4, 2
//         );
//         let anchor_group = AnchorGroup::new(&ref_seq, &qry_seq, &index, aligner.kmer, &aligner.emp_kmer, &aligner.scores, &aligner.cutoff).unwrap();
//         println!("{:?}", anchor_group.anchors);
//     }

//     #[test]
//     fn test_alignment() {
//         let test_data = test_data::get_test_data();
//         let seqs = test_data[1].clone();
//         let ref_seq = seqs.0.as_bytes();
//         let qry_seq = seqs.1.as_bytes();
//         let index = super::super::Reference::fmindex(&ref_seq);
//         let aligner = super::super::tests::test_aligner(
// 0.6, 100, 3, 4, 2
//         );
//         let mut anchor_group = AnchorGroup::new(&ref_seq, &qry_seq, &index, aligner.kmer, &aligner.emp_kmer, &aligner.scores, &aligner.cutoff).unwrap();
//         let alignment_res = anchor_group.alignment();
//         println!("{:?}", alignment_res);
//     }
// }