use std::env;
use std::time::Duration;

pub struct Config {
    pub tcp_server_addr: String,
    pub udp_server_addr: String,
    pub scenery_count: usize,
    pub tick_rate: Duration,
}

impl Config {
    pub fn from_env() -> Self {
        let tcp_server_addr =
            env::var("TCP_SERVER_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
        let udp_server_addr =
            env::var("UDP_SERVER_ADDR").unwrap_or_else(|_| "127.0.0.1:8081".to_string());
        let scenery_count = env::var("SCENERY_COUNT")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(10);
        let tick_rate: Duration = Duration::from_nanos(16_666_667); // 60 FPS, 60 Hz
        Self {
            tcp_server_addr,
            udp_server_addr,
            scenery_count,
            tick_rate,
        }
    }
}
