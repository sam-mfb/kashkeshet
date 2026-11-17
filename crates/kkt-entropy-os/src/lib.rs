//! Randomness from the operating system: [`OsEntropy`] implements
//! [`Entropy`](kkt_entropy::Entropy) with the operating system's
//! cryptographic generator.

use kkt_entropy::Entropy;

/// Entropy from the operating system's cryptographic generator.
pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn below(&self, n: usize) -> usize {
        use rand::RngExt;
        rand::rng().random_range(0..n)
    }
}

#[cfg(test)]
mod tests {
    use super::OsEntropy;
    use kkt_entropy::Entropy;

    #[test]
    fn draws_stay_below_the_bound() {
        assert!((0..200).all(|_| OsEntropy.below(7) < 7));
        assert_eq!(OsEntropy.below(1), 0);
    }
}
