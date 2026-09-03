use alloc::string::ToString;
use embassy_time::Timer;
use esp_radio::wifi::{Config, WifiController, sta::StationConfig};
use heapless::String;
use tracing::{error, info, warn};

#[derive(Debug, Clone)]
pub struct WifiCredentials {
    pub ssid: String<32>,
    pub password: String<64>,
}

impl WifiCredentials {
    pub fn new(ssid: String<32>, password: String<64>) -> Self {
        Self { ssid, password }
    }
}

#[embassy_executor::task]
pub async fn wifi_task(mut controller: WifiController<'static>, creds: WifiCredentials) {
    info!("[WifiTask] Starting Wi-Fi task for SSID: {}", creds.ssid);

    let client_config = Config::Station(
        StationConfig::default()
            .with_ssid(creds.ssid.to_string())
            .with_password(creds.password.to_string()),
    );

    if let Err(e) = controller.set_config(&client_config) {
        warn!("[WifiTask] Failed to set Wi-Fi config: {:?}", e);
        return;
    }

    loop {
        info!("[WifiTask] Connecting to Wi-Fi...");
        match controller.connect_async().await {
            Ok(_) => {
                info!("[WifiTask] Connected to Wi-Fi!");
                // Hang here until hardware reports disconnection
                let _ = controller.wait_for_disconnect_async().await;
                error!("[WifiTask] Wi-Fi connection lost!");
            }
            Err(e) => {
                warn!("[WifiTask] Wi-Fi connection attempt failed: {:?}", e);
            }
        }

        info!("[WifiTask] Waiting 30 seconds before reconnecting...");
        Timer::after_secs(30).await;
    }
}
