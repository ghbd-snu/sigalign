//! Dropout alignment core
pub mod anchor;
pub mod dropout_wfa;

use anchor::AnchorGroup;

use fm_index::converter::RangeConverter;
use fm_index::suffix_array::{SuffixOrderSampledArray, SuffixOrderSampler};
use fm_index::FMIndex;
type FmIndex = FMIndex<u8, RangeConverter<u8>, SuffixOrderSampledArray>;

type SeqeunceLength = usize;

const FM_SUFFIX_LEVEL: usize = 2;

#[derive(Debug)]
pub struct Aligner {
    cutoff: Cutoff,
    kmer: usize,
    scores: Scores,
    emp_kmer: EmpKmer,
    using_cached_wf: bool,
    get_minimum_penalty: bool,
}

// Alignment Result: (operations, penalty)
type AlignmentResult = Vec<(Vec<Operation>, usize)>;

impl Aligner {
    pub fn new(score_per_length: f64, minimum_length: usize, mismatch_penalty: usize, gapopen_penalty: usize, gapext_penalty: usize, using_cached_wf: bool, get_minimum_penalty: bool) -> Self {
        let emp_kmer = EmpKmer::new(mismatch_penalty, gapopen_penalty, gapext_penalty);
        let kmer = Self::kmer_calculation(score_per_length, minimum_length, &emp_kmer);
        Self {
            cutoff: Cutoff {
                score_per_length: score_per_length,
                minimum_length: minimum_length,
            },
            kmer: kmer,
            scores: (mismatch_penalty, gapopen_penalty, gapext_penalty),
            emp_kmer: emp_kmer,
            using_cached_wf: using_cached_wf,
            get_minimum_penalty: get_minimum_penalty,
        }
    }
    fn kmer_calculation(score_per_length: f64, minimum_length: usize, emp_kmer: &EmpKmer) -> usize {
        let mut i: usize = 1;
        let mut kmer_size: f64;
        loop {
            kmer_size = (((minimum_length+2) as f64/(2*i) as f64) - 1_f64).ceil();
            if (i*(emp_kmer.odd + emp_kmer.even)) as f64 > score_per_length * 2_f64 * (((i+1) as f64)*kmer_size-1_f64) {
                break;
            } else {
                i += 1;
            }
        }
        kmer_size as usize
    }
    pub fn perform_with_sequence(&self, ref_seq: &[u8] , qry_seq: &[u8]) -> Option<AlignmentResult> {
        let index = Reference::fmindex(&ref_seq);
        let result = match AnchorGroup::new(ref_seq, qry_seq, &index, self.kmer, &self.emp_kmer, &self.scores, &self.cutoff) {
            Some(mut anchor_group) => {
                anchor_group.alignment(self.using_cached_wf);
                Some(anchor_group.get_result(self.get_minimum_penalty))
            },
            None => None,
        };
        result
    }
    pub fn perform_with_index<T: AsRef<[u8]>>(&self, reference: &Reference<T> , qry_seq: &[u8]) -> Option<AlignmentResult> {
        let result = match AnchorGroup::new(reference.sequence.as_ref(), qry_seq, &reference.index, self.kmer, &self.emp_kmer, &self.scores, &self.cutoff) {
            Some(mut anchor_group) => {
                anchor_group.alignment(self.using_cached_wf);
                Some(anchor_group.get_result(self.get_minimum_penalty))
            },
            None => None,
        };
        result
    }
}

pub struct Reference<T: AsRef<[u8]>>{
    sequence: T,
    index: FmIndex
}
impl<T: AsRef<[u8]>> Reference<T> {
    fn new(sequence: T) -> Self {
        let fm_index =  Self::fmindex(&sequence);
        Self {
            sequence: sequence,
            index: fm_index,
        }
    }
    fn fmindex(sequence: &T) -> FmIndex {
        let seq = sequence.as_ref().iter().cloned().collect();
        // TODO: change the input ASCII code range
        let converter = RangeConverter::new(b'A', b'T');
        let sampler = SuffixOrderSampler::new().level(FM_SUFFIX_LEVEL);
        FMIndex::new(seq, converter, sampler)
    }
}

type Scores = (usize, usize, usize);

#[derive(Debug)]
pub struct Cutoff {
    score_per_length: f64,
    minimum_length: usize,
}

#[derive(Debug)]
pub struct EmpKmer {
    odd: usize,
    even: usize,
}

impl EmpKmer {
    fn new(mismatch_penalty: usize, gapopen_penalty: usize, gapext_penalty: usize) -> Self {
        let mo: usize;
        let me: usize;
        if mismatch_penalty <= gapopen_penalty + gapext_penalty {
            mo = mismatch_penalty;
            if mismatch_penalty * 2 <= gapopen_penalty + (gapext_penalty * 2) {
                me = mismatch_penalty;
            } else {
                me = gapopen_penalty + (gapext_penalty * 2) - mismatch_penalty;
            }
        } else {
            mo = gapopen_penalty + gapext_penalty;
            me = gapext_penalty;
        }
        Self {
            odd: mo,
            even: me,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Operation {
    Match,
    Subst,
    Ins,
    Del,
    RefClip(SeqeunceLength),
    QryClip(SeqeunceLength),
}
