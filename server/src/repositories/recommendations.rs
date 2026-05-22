use async_trait::async_trait;
#[cfg(test)]
use mockall::automock;
use serde::{Deserialize, Serialize};
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoRecommendation {
    pub video_id: String,
    pub score: i64,
}

#[derive(Debug, Deserialize)]
struct RecommendationRow {
    video_id: String,
    score: i64,
}

/// Graph-traversal recommendations for watch rooms. Surfaces videos watched
/// by other members of the servers this user is in, that the user hasn't
/// watched yet, ranked by how many distinct co-members have watched them.
#[cfg_attr(test, automock)]
#[async_trait]
pub trait RecommendationsRepo: Send + Sync {
    /// Up to `limit` video recommendations for this user. Excludes videos
    /// already in the user's `watched` edges. Empty when the user has no
    /// server peers with `watched` history.
    async fn for_user(
        &self,
        user_id: &str,
        limit: u32,
    ) -> Result<Vec<VideoRecommendation>, AppError>;
}

pub struct SurrealRecommendationsRepo {
    db: Surreal<Client>,
}

impl SurrealRecommendationsRepo {
    pub fn new(db: Surreal<Client>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl RecommendationsRepo for SurrealRecommendationsRepo {
    async fn for_user(
        &self,
        user_id: &str,
        limit: u32,
    ) -> Result<Vec<VideoRecommendation>, AppError> {
        // Two-hop traversal expressed as a single SELECT with inlined
        // subqueries so reordering can't silently change which response slot
        // holds the result (the prior multi-statement form needed `q.take(3)`
        // and failed open if the statement count shifted).
        //
        //   $user --member_of--> server <--member_of-- peers
        //   peers --watched--> media (group by video_id, rank by frequency)
        //
        // Filtered to exclude `$user`'s own edges and any video they've seen.
        // All inputs are bound — no string interpolation.
        let user = surrealdb::RecordId::from(("user", user_id));
        let mut q = self
            .db
            .query(
                "SELECT video_id, count() AS score FROM watched \
                 WHERE in IN ( \
                     SELECT VALUE <-member_of<-user FROM ( \
                         SELECT VALUE ->member_of->server FROM $user \
                     ) \
                 ) \
                 AND in != $user \
                 AND video_id NOTINSIDE ( \
                     SELECT VALUE video_id FROM watched WHERE in = $user \
                 ) \
                 GROUP BY video_id \
                 ORDER BY score DESC \
                 LIMIT $limit;",
            )
            .bind(("user", user))
            .bind(("limit", limit))
            .await?;
        let rows: Vec<RecommendationRow> = q.take(0)?;
        Ok(rows
            .into_iter()
            .map(|r| VideoRecommendation {
                video_id: r.video_id,
                score: r.score,
            })
            .collect())
    }
}
