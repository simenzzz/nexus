use crate::error::AppError;

/// Feed ranking: weight posts by graph distance from the viewer.
/// Closer connections surface higher in the feed.

pub async fn get_ranked_feed() -> Result<Vec<String>, AppError> {
    // TODO: Traverse social graph to rank posts by proximity
    Ok(vec![])
}
