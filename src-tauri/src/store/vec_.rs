//! Embedded vector index.
//!
//! Embeddings are stored as little-endian f32 BLOBs in the `embeddings` table
//! (SQLite — fully embedded in the binary, no external service). Retrieval is
//! brute-force cosine similarity, which is exact and comfortably fast at
//! personal scale (a few thousand vectors × ≤ 768 dims = a few MB in memory).
//!
//! Vectors are normalised on write, so "cosine" reduces to a dot product.
//! `model` + `dim` are stored per row so that a model upgrade can never
//! produce bogus cross-model similarity scores.

use rusqlite::{params, Connection, Result as SqlResult};

use crate::error::Result;

/// A kNN hit: the referenced id plus cosine similarity in [-1, 1].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub ref_kind: String,
    pub ref_id: String,
    pub score: f64,
}

/// Normalise a vector in place; returns the norm.
fn normalise(v: &mut [f32]) -> f32 {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
    norm
}

fn to_blob(vec: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vec.len() * 4);
    for x in vec {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

fn from_blob(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Store a vector. Replaces any previous vector for `(ref_kind, ref_id, model)`.
pub fn insert(conn: &Connection, ref_kind: &str, ref_id: &str, model: &str, vec: &[f32]) -> Result<()> {
    let mut v = vec.to_vec();
    normalise(&mut v);
    conn.execute(
        "INSERT OR REPLACE INTO embeddings (id, ref_kind, ref_id, dim, model, vec)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            format!("{ref_kind}:{ref_id}:{model}"),
            ref_kind,
            ref_id,
            v.len() as i64,
            model,
            to_blob(&v)
        ],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

/// Cosine-similarity kNN against all rows written with `model`
/// (optionally filtered to one `ref_kind`).
pub fn knn(
    conn: &Connection,
    model: &str,
    ref_kind: Option<&str>,
    query: &[f32],
    k: usize,
) -> Result<Vec<Hit>> {
    let mut query = query.to_vec();
    normalise(&mut query);

    let sql = match ref_kind {
        Some(_) => "SELECT ref_kind, ref_id, vec FROM embeddings WHERE model = ?1 AND ref_kind = ?2",
        None => "SELECT ref_kind, ref_id, vec FROM embeddings WHERE model = ?1",
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    // A function-scope binding of the kind, so the ToSql reference it yields
    // outlives `args` (arm-locals would not).
    let kind_str: &str = ref_kind.unwrap_or("");
    let args: Vec<&dyn rusqlite::ToSql> = if ref_kind.is_some() {
        vec![&model as &dyn rusqlite::ToSql, &kind_str as &dyn rusqlite::ToSql]
    } else {
        vec![&model as &dyn rusqlite::ToSql]
    };

    let row_iter = stmt
        .query_map(args.as_slice(), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    let blobs: Vec<(String, String, Vec<u8>)> = row_iter
        .collect::<SqlResult<Vec<_>>>()
        .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    let rows: Vec<(String, String, Vec<f32>)> = blobs
        .into_iter()
        .map(|(kind, id, blob)| (kind, id, from_blob(&blob)))
        .collect();

    let mut hits: Vec<Hit> = rows
        .into_iter()
        .filter(|(_, _, v)| v.len() == query.len())
        .map(|(kind, id, v)| {
            let dot: f32 = v.iter().zip(&query).map(|(a, b)| a * b).sum();
            Hit {
                ref_kind: kind,
                ref_id: id,
                score: dot as f64,
            }
        })
        .collect();
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(k);
    Ok(hits)
}

/// Delete all vectors belonging to a reference.
pub fn delete_ref(conn: &Connection, ref_kind: &str, ref_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM embeddings WHERE ref_kind = ?1 AND ref_id = ?2",
        params![ref_kind, ref_id],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    Ok(())
}

/// Count stored vectors (for status / reindex reporting).
pub fn count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0)
}

/// Re-embed everything from a text source: drop all vectors for a model and
/// rebuild. Returns the number of rows written.
pub fn rebuild_all<F>(conn: &Connection, model: &str, mut source: F) -> Result<u32>
where
    F: FnMut() -> SqlResult<Vec<(String, String, Vec<f32>)>>,
{
    conn.execute(
        "DELETE FROM embeddings WHERE model = ?1",
        params![model],
    )
    .map_err(|e| crate::error::Error::Store(e.to_string()))?;
    let mut n = 0u32;
    while let Ok(batch) = source() {
        if batch.is_empty() {
            break;
        }
        for (kind, id, v) in batch {
            insert(conn, &kind, &id, model, &v)?;
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn conn() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        super::super::db::init_schema(&mut c).unwrap();
        c
    }

    #[test]
    fn knn_ranks_similar_vectors_first() {
        let c = conn();
        insert(&c, "topic", "a", "m", &[1.0, 0.0, 0.0, 0.0]).unwrap();
        insert(&c, "topic", "b", "m", &[0.5, 0.5, 0.0, 0.0]).unwrap();
        insert(&c, "topic", "c", "m", &[0.0, 0.0, 0.0, 1.0]).unwrap();
        let hits = knn(&c, "m", Some("topic"), &[1.0, 0.2, 0.0, 0.0], 3).unwrap();
        assert_eq!(hits[0].ref_id, "a");
        assert!(hits[0].score > hits[1].score);
        assert!(hits[2].score < 0.1);
    }

    #[test]
    fn models_never_mix() {
        let c = conn();
        insert(&c, "topic", "a", "m1", &[1.0, 0.0]).unwrap();
        insert(&c, "topic", "b", "m2", &[1.0, 0.0]).unwrap();
        let hits = knn(&c, "m1", Some("topic"), &[1.0, 0.0], 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].ref_id, "a");
    }
}
