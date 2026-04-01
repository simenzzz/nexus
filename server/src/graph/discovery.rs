use crate::error::AppError;

/// Server discovery: recommend servers based on friend overlap.
/// Traverses user -> friends_with -> user -> member_of -> server.

pub async fn discover_servers() -> Result<Vec<String>, AppError> {
    // TODO: Rank servers by number of friends who are members
    Ok(vec![])
}
