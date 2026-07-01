use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("embedding failed: {0}")]
    Failed(String),
}

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
    fn dim(&self) -> usize;
}

/// Cosine similarity. Returns NaN on a dimension mismatch (caller treats NaN as "skip"),
/// and 0.0 when either vector has zero magnitude.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::NAN;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Deterministic, dependency-free embedder for tests: identical text → identical vector
/// (cosine 1.0), distinct text → near-orthogonal vectors. NOT semantic — paraphrase
/// matching is only validated by the live `#[ignore]` test against the real model.
pub struct StubEmbedder {
    dim: usize,
}

impl StubEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
    pub fn d384() -> Self {
        Self { dim: 384 }
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[async_trait]
impl Embedder for StubEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0f32; self.dim];
                for (i, slot) in v.iter_mut().enumerate() {
                    let h = fnv1a(format!("{i}:{t}").as_bytes());
                    *slot = ((h % 2000) as f32 / 1000.0) - 1.0; // [-1, 1)
                }
                let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if n > 0.0 {
                    for x in &mut v {
                        *x /= n;
                    }
                }
                v
            })
            .collect())
    }
    fn dim(&self) -> usize {
        self.dim
    }
}

#[cfg(feature = "onnx")]
pub struct FastEmbedEmbedder {
    model: std::sync::Mutex<fastembed::TextEmbedding>,
    dim: usize,
}

#[cfg(feature = "onnx")]
impl FastEmbedEmbedder {
    /// Load BGE-Small-EN-v1.5 (384-dim). Downloads the ONNX model to the cache dir on
    /// first use (network required once); cached thereafter. Returns Err offline-with-no-cache.
    pub fn new(cfg: &crate::config::MemoryConfig) -> Result<Self, EmbedError> {
        use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
        let mut opts = InitOptions::new(EmbeddingModel::BGESmallENV15);
        if let Some(dir) = &cfg.model_cache_dir {
            opts = opts.with_cache_dir(dir.clone());
        }
        let model = TextEmbedding::try_new(opts).map_err(|e| EmbedError::Failed(e.to_string()))?;
        Ok(Self {
            model: std::sync::Mutex::new(model),
            dim: 384,
        })
    }
}

#[cfg(feature = "onnx")]
#[async_trait]
impl Embedder for FastEmbedEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let owned: Vec<String> = texts.to_vec();
        let res = {
            let guard = self.model.lock().unwrap();
            guard.embed(owned, None)
        };
        res.map_err(|e| EmbedError::Failed(e.to_string()))
    }
    fn dim(&self) -> usize {
        self.dim
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn identical_text_is_cosine_one_distinct_is_low() {
        let e = StubEmbedder::d384();
        let v = e
            .embed(&["alpha".into(), "alpha".into(), "totally different".into()])
            .await
            .unwrap();
        assert_eq!(v[0].len(), 384);
        assert!((cosine(&v[0], &v[1]) - 1.0).abs() < 1e-5, "same text → 1.0");
        assert!(cosine(&v[0], &v[2]) < 0.5, "distinct text → low similarity");
    }

    #[test]
    fn cosine_dimension_mismatch_is_nan() {
        assert!(cosine(&[1.0, 0.0], &[1.0, 0.0, 0.0]).is_nan());
    }
}
