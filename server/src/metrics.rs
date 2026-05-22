use metrics::{counter, gauge, histogram};
use std::time::Instant;

pub fn record_ws_connect() {
    counter!("ws_connected_total").increment(1);
    gauge!("ws_connected_clients").increment(1.0);
}

pub fn record_ws_disconnect() {
    counter!("ws_disconnected_total").increment(1);
    gauge!("ws_connected_clients").decrement(1.0);
}

pub fn record_message_sent() {
    counter!("messages_sent_total").increment(1);
}

pub fn record_room_created() {
    counter!("rooms_created_total").increment(1);
    gauge!("active_rooms").increment(1.0);
}

pub fn record_room_closed() {
    counter!("rooms_closed_total").increment(1);
    gauge!("active_rooms").decrement(1.0);
}

pub fn record_db_query(duration_ms: f64) {
    histogram!("db_query_duration_ms").record(duration_ms);
}

pub fn record_rate_limit_rejection() {
    counter!("rate_limit_rejections_total").increment(1);
}

pub fn record_rec_breaker_trip() {
    counter!("rec_breaker_trips_total").increment(1);
}

pub fn record_presence_flap_suppressed() {
    counter!("presence_flap_suppressed_total").increment(1);
}

pub struct DbQueryTimer {
    start: Instant,
}

impl DbQueryTimer {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn record(self) {
        let elapsed = self.start.elapsed().as_secs_f64() * 1000.0;
        record_db_query(elapsed);
    }
}

impl Default for DbQueryTimer {
    fn default() -> Self {
        Self::new()
    }
}
