//! Randomness from the operating system: [`OsEntropy`], kkt-entropy's
//! [`Entropy`](kkt_entropy::Entropy) from the operating system's
//! cryptographic generator. Behind `real`.

#[cfg(feature = "real")]
use kkt_entropy::Entropy;

/// Entropy from the operating system's cryptographic generator.
#[cfg(feature = "real")]
pub struct OsEntropy;

#[cfg(feature = "real")]
impl Entropy for OsEntropy {
    fn below(&self, n: usize) -> usize {
        use rand::RngExt;
        rand::rng().random_range(0..n)
    }
}

#[cfg(all(test, feature = "real"))]
mod tests {
    use super::OsEntropy;
    use kkt_entropy::Entropy;

    #[test]
    fn draws_stay_below_the_bound() {
        assert!((0..200).all(|_| OsEntropy.below(7) < 7));
        assert_eq!(OsEntropy.below(1), 0);
    }
}
