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

    /// How many characters kkt's IDs and tokens are drawn from: lowercase
    /// ASCII letters and digits.
    const ID_CHARS: usize = 36;

    /// Picks the first word for every handle, so every handle clashes, but
    /// draws IDs and tokens normally. Handle words come from lists longer than
    /// the ID alphabet, which is how the two kinds of draw are told apart.
    pub struct SameHandleEntropy(pub SeqEntropy);

    impl Entropy for SameHandleEntropy {
        fn below(&self, n: usize) -> usize {
            if n > ID_CHARS { 0 } else { self.0.below(n) }
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
    use super::testing::{SameHandleEntropy, SeqEntropy};

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
    fn same_handle_entropy_picks_the_first_word_but_draws_ids_normally() {
        let same = SameHandleEntropy(SeqEntropy::new(5));
        let plain = SeqEntropy::new(5);
        assert_eq!(same.below(100), 0);
        assert_eq!(same.below(36), plain.below(36));
    }
}
