//! Randomness, as a contract: [`Entropy`] gives uniformly chosen numbers,
//! and draws random characters from an alphabet the caller gives.
//!
//! Randomness is injected through [`Entropy`] so the rest of the system can
//! be tested with a predictable source. The stand-ins, behind `testing`,
//! are [`testing`]'s and [`SeededEntropy`].

/// A source of random numbers.
pub trait Entropy: Send + Sync {
    /// Returns a uniformly chosen number in `0..n`. `n` is never zero.
    fn below(&self, n: usize) -> usize;

    /// `len` characters, each chosen uniformly from `alphabet`, which is
    /// never empty.
    fn random_chars(&self, alphabet: &[u8], len: usize) -> String {
        (0..len)
            .map(|_| alphabet[self.below(alphabet.len())] as char)
            .collect()
    }
}

#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use super::Entropy;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A predictable entropy source for tests (a simple linear congruential generator).
    pub struct SeqEntropy(AtomicU64);

    impl SeqEntropy {
        pub fn new(seed: u64) -> Self {
            Self(AtomicU64::new(seed))
        }
    }

    impl Entropy for SeqEntropy {
        fn below(&self, n: usize) -> usize {
            let next = self
                .0
                .load(Ordering::Relaxed)
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0.store(next, Ordering::Relaxed);
            ((next >> 33) as usize) % n
        }
    }

    /// The size of an alphabet of lowercase ASCII letters and digits.
    const ALPHANUMERIC: usize = 36;

    /// Picks the first choice whenever there are more than 36 to choose
    /// from, and draws from its sequence otherwise. So every pick from a
    /// long list, such as a list of words, is the same, while characters
    /// drawn from a lowercase alphanumeric alphabet still vary.
    pub struct FirstOfLongListsEntropy(pub SeqEntropy);

    impl Entropy for FirstOfLongListsEntropy {
        fn below(&self, n: usize) -> usize {
            if n > ALPHANUMERIC { 0 } else { self.0.below(n) }
        }
    }
}

/// Entropy that repeats from a seed: the same seed gives the same numbers in
/// the same order. For simulations such as the web preview; never for tokens
/// or keys. (SplitMix64.)
#[cfg(any(test, feature = "testing"))]
pub struct SeededEntropy(std::sync::atomic::AtomicU64);

#[cfg(any(test, feature = "testing"))]
impl SeededEntropy {
    pub fn new(seed: u64) -> SeededEntropy {
        SeededEntropy(std::sync::atomic::AtomicU64::new(seed))
    }
}

#[cfg(any(test, feature = "testing"))]
impl Entropy for SeededEntropy {
    fn below(&self, n: usize) -> usize {
        const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
        let state = self
            .0
            .fetch_add(GAMMA, std::sync::atomic::Ordering::Relaxed)
            .wrapping_add(GAMMA);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z % n as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::Entropy;
    use super::SeededEntropy;
    use super::testing::{FirstOfLongListsEntropy, SeqEntropy};

    #[test]
    fn seeded_entropy_repeats_from_its_seed() {
        let draws = |seed| {
            let e = SeededEntropy::new(seed);
            (0..20).map(|_| e.below(1000)).collect::<Vec<_>>()
        };
        assert_eq!(draws(7), draws(7));
        assert_ne!(draws(7), draws(8));
        assert!(draws(7).iter().all(|&n| n < 1000));
    }

    #[test]
    fn random_chars_come_from_the_alphabet_given() {
        let e = SeqEntropy::new(1);
        let drawn = e.random_chars(b"xyz", 40);
        assert_eq!(drawn.len(), 40);
        assert!(drawn.bytes().all(|b| b"xyz".contains(&b)), "{drawn}");
        assert_eq!(e.random_chars(b"q", 3), "qqq");
        assert_eq!(e.random_chars(b"xyz", 0), "");
    }

    #[test]
    fn long_lists_get_their_first_choice_and_short_ones_vary() {
        let first = FirstOfLongListsEntropy(SeqEntropy::new(5));
        let plain = SeqEntropy::new(5);
        assert_eq!(first.below(100), 0);
        assert_eq!(first.below(36), plain.below(36));
    }
}
