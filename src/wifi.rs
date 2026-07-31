use alloc::{string::ToString, vec::Vec};
use anyhow::anyhow;
use async_oneshot::oneshot;
use embassy_executor::SendSpawner;
use embassy_futures::select::{Either, select};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, mutex::Mutex};
use embedded_storage::{ReadStorage, Storage};
use esp_bootloader_esp_idf::partitions::FlashRegion;
use esp_radio::wifi::{
    ModeConfig, WifiController, WifiEvent, ap::AccessPointInfo, scan::ScanConfig,
    sta::StationConfig,
};
use esp_storage::FlashStorage;
use heapless::String;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};

static WIFI_CREDS: Mutex<CriticalSectionRawMutex, Option<WifiCredentials>> = Mutex::new(None);

static WIFI_CONTROLLER_COMMAND_CHANNEL: Channel<CriticalSectionRawMutex, WifiControllerCommand, 6> =
    Channel::new();

/// Magic header to identify valid WiFi credentials in flash
const WIFI_STORAGE_MAGIC: u32 = 0x57494649; // "WIFI" in ASCII
const WIFI_STORAGE_VERSION: u8 = 1;
const WIFI_STORAGE_OFFSET: u32 = 0x1000; // Store at 4KB offset in NVS partition
const MAX_CREDENTIALS_SIZE: usize = 256; // Max size for serialized credentials

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct WifiCredentials {
    pub ssid: String<32>,
    pub password: String<64>,
}

impl WifiCredentials {
    pub fn new(ssid: String<32>, password: String<64>) -> Self {
        Self { ssid, password }
    }
}

#[derive(Debug)]
pub enum WifiStorageError {
    SerializationError,
    DeserializationError,
    StorageError,
    InvalidMagic,
    InvalidVersion,
    NoCredentialsFound,
}

pub struct WifiStorageV2 {
    partition: FlashRegion<'static, FlashStorage<'static>>,
}

impl WifiStorageV2 {
    pub fn new(partition: FlashRegion<'static, FlashStorage<'static>>) -> Self {
        Self {
            partition: partition,
        }
    }

    pub async fn save_credentials(
        &mut self,
        creds: &WifiCredentials,
    ) -> Result<(), WifiStorageError> {
        info!("Saving WiFi credentials for SSID: {}", creds.ssid);

        let json =
            serde_json::to_string(&creds).map_err(|_| WifiStorageError::SerializationError)?;
        let json_bytes = json.as_bytes();

        if json_bytes.len() > MAX_CREDENTIALS_SIZE - 6 {
            error!("Credentials too large to store");
            return Err(WifiStorageError::SerializationError);
        }

        let mut buffer = [0u8; MAX_CREDENTIALS_SIZE];
        buffer[0..4].copy_from_slice(&WIFI_STORAGE_MAGIC.to_le_bytes());
        buffer[4] = WIFI_STORAGE_VERSION;
        buffer[5] = json_bytes.len() as u8;
        buffer[6..6 + json_bytes.len()].copy_from_slice(json_bytes);

        self.partition
            .write(WIFI_STORAGE_OFFSET, &buffer)
            .map_err(|_| WifiStorageError::StorageError)?;
        info!("WiFi credentials saved successfully");
        Ok(())
    }

    pub async fn load_credentials(&mut self) -> Result<WifiCredentials, WifiStorageError> {
        info!("Loading WiFi credentials from storage");

        let mut buffer = [0u8; MAX_CREDENTIALS_SIZE];

        self.partition
            .read(WIFI_STORAGE_OFFSET, &mut buffer)
            .map_err(|_| WifiStorageError::StorageError)?;

        let magic = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
        if magic != WIFI_STORAGE_MAGIC {
            warn!("Invalid magic header: 0x{:08x}", magic);
            return Err(WifiStorageError::InvalidMagic);
        }

        let version = buffer[4];
        if version != WIFI_STORAGE_VERSION {
            warn!("Invalid version: {}", version);
            return Err(WifiStorageError::InvalidVersion);
        }

        let data_len = buffer[5] as usize;
        if data_len == 0 || data_len > MAX_CREDENTIALS_SIZE - 6 {
            warn!("Invalid data length: {}", data_len);
            return Err(WifiStorageError::NoCredentialsFound);
        }

        let json_slice = &buffer[6..6 + data_len];
        let json_str =
            core::str::from_utf8(json_slice).map_err(|_| WifiStorageError::DeserializationError)?;

        let credentials: WifiCredentials =
            serde_json::from_str(json_str).map_err(|_| WifiStorageError::DeserializationError)?;

        info!(
            "WiFi credentials loaded successfully for SSID: {}",
            credentials.ssid
        );

        Ok(credentials)
    }

    pub async fn clear_credentials(&mut self) -> Result<(), WifiStorageError> {
        info!("Clearing WiFi credentials");

        let buffer = [0u8; MAX_CREDENTIALS_SIZE];

        self.partition
            .write(WIFI_STORAGE_OFFSET, &buffer)
            .map_err(|_| WifiStorageError::StorageError)?;

        info!("WiFi credentials cleared");
        Ok(())
    }

    pub async fn has_credentials(&mut self) -> bool {
        let mut buffer = [0u8; 4];

        if self
            .partition
            .read(WIFI_STORAGE_OFFSET, &mut buffer)
            .is_err()
        {
            return false;
        }

        let magic = u32::from_le_bytes(buffer);
        magic == WIFI_STORAGE_MAGIC
    }
}

// =====================
// CONNECT TO WIFI (single attempt)
// =====================
async fn connect_to_wifi(controller: &mut WifiController<'static>) -> Result<(), anyhow::Error> {
    if !matches!(controller.is_started(), Ok(true)) {
        let client_config = {
            let creds_guard = WIFI_CREDS.lock().await;

            let creds = creds_guard
                .as_ref()
                .ok_or(anyhow!("No wifi credentials found"))?;

            ModeConfig::Station(
                StationConfig::default()
                    .with_ssid(creds.ssid.to_string())
                    .with_password(creds.password.to_string()),
            )
        };

        controller.set_config(&client_config).unwrap();
        info!("Starting Wi-Fi with current credentials...");

        controller.start_async().await.unwrap();
        info!("Wi-Fi started");
    }

    info!("Connection to wifi...");

    controller.connect_async().await?;
    info!("Connected to wifi!");
    Ok(())
}

// =====================
// CONNECT TO WIFI (with retries)
// =====================
async fn connect_to_wifi_with_retries(
    controller: &mut WifiController<'static>,
    max_attempts: u8,
) -> Result<(), anyhow::Error> {
    let mut last_err = None;
    for attempt in 1..=max_attempts {
        match connect_to_wifi(controller).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                warn!(
                    "Wi-Fi connect attempt {}/{} failed: {}",
                    attempt, max_attempts, e
                );
                last_err = Some(e);
                if attempt < max_attempts {
                    embassy_time::Timer::after_secs(2).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("connect failed, no error captured")))
}

// =====================
// MAIN CONNECTION LOOP
// =====================
#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    info!("Start connection task");
    info!("Device capabilities: {:?}", controller.ap_info());

    loop {
        let command = WIFI_CONTROLLER_COMMAND_CHANNEL.receive();
        let disconnect = controller.wait_for_event(WifiEvent::StationDisconnected);

        match select(command, disconnect).await {
            Either::First(command) => match command {
                WifiControllerCommand::ConnectToWifi(creds, mut response_consumer) => {
                    *WIFI_CREDS.lock().await = Some(creds);
                    if controller.is_started().unwrap_or(false) {
                        let _ = controller.stop(); // Ignore errors on stop
                        info!("Stopped Wi-Fi to apply new credentials");
                    }

                    let result = self::connect_to_wifi_with_retries(&mut controller, 3).await;
                    let outcome = result.map(|_| ()).map_err(|e| {
                        warn!("Failed connect to wifi after retries: {}", e);
                    });

                    response_consumer.send(outcome).unwrap_or_else(|_| {
                        warn!("Receiver was dropped before connect result was sent");
                    });
                }
                WifiControllerCommand::GetAvailableNetworks(mut response_consumer) => {
                    let found_networks = controller
                        .scan_with_config_async(ScanConfig::default())
                        .await;
                    if let Ok(networks) = found_networks {
                        response_consumer.send(networks).unwrap_or_else(|_| {
                            warn!("Receiver was dropped before networks were sent");
                        });
                    }
                }
                WifiControllerCommand::GetWifiStatus(mut response_consumer) => {
                    let status = if controller.is_connected().unwrap() {
                        WifiStatusInternal::Connected
                    } else {
                        WifiStatusInternal::Disconnected
                    };
                    response_consumer.send(status).unwrap();
                }
            },
            Either::Second(_) => {
                info!("Wi-Fi disconnected, queueing reconnect...");
                self::connect_to_wifi_with_retries(&mut controller, 3)
                    .await
                    .unwrap_or_else(|e| {
                        warn!("Failed connect to wifi after retries: {}", e);
                    });
            }
        };
    }
}

pub enum WifiControllerCommand {
    GetAvailableNetworks(async_oneshot::Sender<Vec<AccessPointInfo>>),
    ConnectToWifi(WifiCredentials, async_oneshot::Sender<Result<(), ()>>),
    GetWifiStatus(async_oneshot::Sender<WifiStatusInternal>),
}

#[derive(Clone)]
pub struct WifiManager {}

impl WifiManager {
    pub async fn new(spawner: SendSpawner, wifi_controller: WifiController<'static>) -> Self {
        spawner
            .spawn(connection(wifi_controller))
            .expect("Error launching wifi task");
        Self {}
    }

    pub async fn get_networks(&self) -> Result<Vec<AccessPointInfo>, anyhow::Error> {
        let (s, r) = oneshot::<Vec<AccessPointInfo>>();
        WIFI_CONTROLLER_COMMAND_CHANNEL
            .send(WifiControllerCommand::GetAvailableNetworks(s))
            .await;

        Ok(r.await.map_err(|_| {
            anyhow!("Failed to get networks nearby, channel was closed when tried to receive")
        })?)
    }

    pub async fn get_wifi_status(&self) -> WifiStatusInternal {
        let (s, r) = oneshot::<WifiStatusInternal>();
        WIFI_CONTROLLER_COMMAND_CHANNEL
            .send(WifiControllerCommand::GetWifiStatus(s))
            .await;

        r.await.unwrap()
    }

    /// Sends the connect command and blocks until the connection task has
    /// finished trying (including retries). Returns Err(()) if all attempts
    /// failed.
    pub async fn connect(&self, credentials: WifiCredentials) -> Result<(), ()> {
        let (s, r) = oneshot::<Result<(), ()>>();
        WIFI_CONTROLLER_COMMAND_CHANNEL
            .send(WifiControllerCommand::ConnectToWifi(credentials, s))
            .await;

        r.await.unwrap_or(Err(()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum WifiStatusInternal {
    Connected,
    Disconnected,
}
