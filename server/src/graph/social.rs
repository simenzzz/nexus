use crate::error::AppError;

/// Social graph operations: friends, mutual friends, friend suggestions.
/// Uses SurrealDB graph traversal (->friends_with->user).

pub async fn get_mutual_friends() -> Result<Vec<String>, AppError> {
    // TODO: SELECT ->friends_with->user FROM $user
    //       WHERE ->friends_with->user->friends_with CONTAINS $other
    Ok(vec![])
}

pub async fn get_friend_suggestions() -> Result<Vec<String>, AppError> {
    // TODO: 2-hop traversal for friends-of-friends, excluding existing friends
    Ok(vec![])
}
