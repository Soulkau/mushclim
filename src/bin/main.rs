#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

extern crate alloc;
use bt_hci::controller::ExternalController;
use esp_storage::FlashStorage;
use ivy::storage::{StorageKey, StorageModule};
use sequential_storage::map::MapConfig;
use core::fmt::Write;
use driverse::am2301::Am2301;
use driverse::relay::Relay;
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_net::{Runner, StackResources};
use embassy_time::{Ticker, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Output, OutputConfig};
use esp_hal::i2c::master::{Config, I2c};
use esp_hal::rng::{Rng, Trng, TrngSource};
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use esp_radio::wifi::WifiDevice;
use ivy::{count, init_storage};
use ivy::declare_topics;
use ivy::mqtt::{MqttHandle, MqttModule, Subscription};
use mushclim::humidifier::Humidifier;
use mushclim::metrics::MetricManager;
use mushclim::timer::CycleTimer;
use mushclim::wifi::{WifiCredentials, wifi_task};

use heapless::String;
use mushclim::exhaust::ExhaustManager;
use mushclim::lcd::{LCD_ADDRESS, Lcd, RGB_ADDRESS, TextAlign, WriteSettings};
use mushclim::measurements::{
    MeasurementError, MeasurementManager, MeasurementProvider, Measurements,
};
use mushclim::{MushclimConfig, MushclimConfigDto, MushclimStats, mk_static};

macro_rules! create_relay {
    ($pin:expr) => {
        Relay::new(
            Output::new($pin, esp_hal::gpio::Level::High, OutputConfig::default()),
            driverse::relay::ActiveLevel::Low,
            false,
        )
        .unwrap()
    };
}

const CONF_KEY: StorageKey = StorageKey::new(101);
// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    // esp_println::logger::init_logger(log::LevelFilter::Debug);
    // Allocate Heap for Radio/COEX
    esp_alloc::heap_allocator!(#[unsafe(link_section = ".dram2_uninit")] size: 65536);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    ivy::logger::init_mqtt_logger();

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
    tracing::info!("Mushclim booted");
    // Wi-Fi and Bluetooth hardware init
    let (wifi_controller, interfaces) = esp_radio::wifi::new(peripherals.WIFI, Default::default())
        .expect("Failed to initialize Wi-Fi controller");

    let transport = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let ble_controller = ExternalController::<BleConnector, 20>::new(transport);

    let wifi_interface = interfaces.station;
    let storage = init_storage!(
        FlashStorage,
        FlashStorage::new(peripherals.FLASH),
        MapConfig::new(0x9000..0xF000)
    );
    // let device_meta = DeviceMetadata::load(&storage, talky::device::DeviceType::MushClimate).await;
    
    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let creds = WifiCredentials::new(
        env!("WIFI_SSID").try_into().unwrap(),
        env!("WIFI_PASS").try_into().unwrap(),
    );
    spawner.must_spawn(wifi_task(wifi_controller, creds));

    // Init network stack
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        embassy_net::Config::dhcpv4(Default::default()),
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    spawner.must_spawn(net_task(runner));
    let _ = TrngSource::new(peripherals.RNG, peripherals.ADC1);
    let trng = Trng::try_new().unwrap();
    let (handles, config_sub) = declare_topics! {
        config => "mushclim/config" : MushclimConfigDto
    };
    let config_sub = config_sub.0;
    let mqtt_handle =
        ivy::actor!(spawner, MqttModule<Trng, 312, 1>, MqttModule::new(stack, trng, handles));

    let mut out = Output::new(
        peripherals.GPIO4,
        esp_hal::gpio::Level::Low,
        OutputConfig::default().with_drive_mode(esp_hal::gpio::DriveMode::OpenDrain),
    )
    .into_flex();
    out.set_input_enable(true);
    let am2301 = Am2301::new(out);
    let lcd: Option<Lcd> = match I2c::new(peripherals.I2C0, Config::default()) {
        Ok(i2c) => {
            tracing::info!("Here");
            let mut lcd = Lcd::new(
                i2c.with_scl(peripherals.GPIO23)
                    .with_sda(peripherals.GPIO22)
                    .into_async(),
                LCD_ADDRESS,
                RGB_ADDRESS,
            );
            tracing::info!("LCD initialized");
            match lcd.init().await {
                Ok(()) => Some(lcd),
                Err(_) => None,
            }
        }
        Err(_) => None,
    };
    let disco = create_relay!(peripherals.GPIO5);
    let fan = create_relay!(peripherals.GPIO6);

    let lights = create_relay!(peripherals.GPIO0); //Those all are on by default

    let measurement_manager = MeasurementManager::new(am2301);
    let config: MushclimConfig = storage.get::<MushclimConfigDto>(CONF_KEY).await.unwrap_or(MushclimConfigDto::default()).as_local();
    let exhaust = ExhaustManager::new(fan, &config);
    let humidifier = Humidifier::new(create_relay!(peripherals.GPIO7), &config);
    let metrics = MetricManager::new(&config);
    let mut mushclim: MushclimApp<'static, _> = MushclimApp {
        lcd,
        config,
        measurement_manager,
        lights,
        humidifier,
        disco,
        exhaust,
        metrics,
        config_sub,
        mqtt_handle, 
        storage
    };
    tracing::info!("Mushclim app was built and running!");
    mushclim.run().await
    /*  CUSTOM APP INITIALIZATION PLACEHOLDER ---
    let app = todo!("Initialize your custom App structure here wrapped in mk_static!");

    let device_id = create_device_id(peripherals.SHA);
    tracing::info!("{device_id:?}");

    storage.save_master_key(&test_signing_key()).await;

    let encore = Encore::new(storage, app, bluetooth_runner, "Chest", device_id).await;

    let master = storage.load_master_key().await;
    tracing::info!("{master:?}");
    tracing::info!("Encore created");

    let main_loop = async {
        loop {
            Timer::after(Duration::from_millis(10000)).await;
        }
    };

    join(encore.run(), main_loop).await;
    unreachable!()
    */
}

// fn create_device_id(sha: SHA<'static>) -> DeviceID {
//     let mac = Efuse::mac_address();
//     let mut source_data = &mac[..];

//     let mut sha = Sha::new(sha);
//     let mut hasher = sha.start::<Sha256>();
//     let mut output = [0u8; 32];

//     while !source_data.is_empty() {
//         source_data = nb::block!(hasher.update(source_data)).unwrap();
//     }

//     hasher.finish(output.as_mut_slice()).unwrap();
//     DeviceID::new(output)
// }

struct MushclimApp<'a, P: MeasurementProvider> {
    measurement_manager: MeasurementManager<P>,
    lcd: Option<Lcd>,
    exhaust: ExhaustManager<'a>,
    config: MushclimConfig,
    lights: Relay<Output<'a>>,
    humidifier: Humidifier<'a>,
    disco: Relay<Output<'a>>,
    metrics: MetricManager,
    config_sub: Subscription<MushclimConfigDto>,
    mqtt_handle: MqttHandle<312>,
    storage: StorageModule<FlashStorage<'static>>
}

impl<'a, P: MeasurementProvider> MushclimApp<'a, P> {
    pub async fn run(&mut self) -> ! {
        self.lights.on();
        self.disco.on();

        tracing::info!("[MushclimApp] Starting sensor calibration");
        self.display_calibrating().await;

        let Ok(_) = self
            .measurement_manager
            .calibrate(&self.config, self.lcd.as_mut())
            .await
        else {
            tracing::error!("[MushclimApp] Failed to calibrate measurements");
            self.enter_safe_mode(4).await
        };

        self.exhaust.timer.ingore_tick();
        let mut ticker = Ticker::every(self.config.loop_delay);

        loop {
            self.tick_step().await;

            // Wait for either the next timer tick or an incoming config update
            match select(ticker.next(), self.config_sub.next()).await {
                Either::First(_) => {
                    // Timer expired normally, loop around for next tick
                }
                Either::Second(config_dto) => {
                    tracing::info!("[MushclimApp] Config update received");
                    self.hard_config_update(config_dto.as_local()).await;
                    self.storage.set(CONF_KEY, &config_dto).await;
                    ticker = Ticker::every(self.config.loop_delay);
                }
            }
        }
    }

    async fn hard_config_update(&mut self, config: MushclimConfig) {
        self.config = config;
        self.exhaust.force_config(&self.config);
        self.metrics.force_config(&self.config); 
        self.humidifier.update_config(&self.config);
        tracing::info!("[MushclimApp] Hard config update performed");
    }

    async fn tick_step(&mut self) {
        self.exhaust.tick().await;
        self.display_fan_state(self.exhaust.log_state()).await;

        match self.acquire_safe_measurement().await {
            Some(measurements) => {
                self.metrics.feed(measurements, &self.mqtt_handle).await;
               
                self.display_measurements(measurements).await;
                self.humidifier
                    .tick(&measurements, self.exhaust.is_turned_on());
                 self.log_current_measurements(measurements, self.humidifier.is_on()).await;
                self.send_stats(measurements, self.exhaust.is_turned_on(), self.humidifier.is_on()).await;
            }
            None => {
                tracing::error!(
                    "[MushclimApp] CRITICAL: Sensor totally failed or reading is permanently erratic"
                );
                self.enter_safe_mode(1).await;
            }
        }
    }

    async fn acquire_safe_measurement(&mut self) -> Option<Measurements> {
        // Attempt up to 5 times to get a stable, validated reading
        for attempt in 1..=self.config.retry_count {
            match self.measurement_manager.get_measurements(&self.config) {
                Ok(stats) => {
                    // It passed hardware check AND history check!
                    return Some(stats);
                }
                Err(MeasurementError::SensorDriver(_e)) => {
                    // "Замерить датчик -> err -> попробовать еще раз (5 попыток)"
                    tracing::warn!("Hardware read error on attempt {}. Retrying...", attempt);
                    Timer::after_secs(2).await;
                }
                Err(MeasurementError::ErraticReading) => {
                    // "сверить новое измерение с историей -> err -> повторить замер и сравнить (5 раз)"
                    tracing::warn!("Spike detected on attempt {}. Re-measuring...", attempt);
                    Timer::after_secs(2).await;
                }
                Err(MeasurementError::NotCalibrated) => {
                    self.enter_safe_mode(3).await;
                }
            }
        }
        None
    }

    async fn enter_safe_mode(&mut self, error_code: u32) -> ! {
        tracing::error!("Entering safe mode");

        self.humidifier.turn_off();
        self.exhaust.reset();
        self.display_safe_mode(error_code).await;

        let mut humidity_timer = CycleTimer::new(
            self.config.safe_humidity_duty_interval,
            self.config.safe_humidity_duty,
        );

        loop {
            self.exhaust.tick().await;

            let humidifier_on = if self.exhaust.is_turned_on() && humidity_timer.duty_started() {
                humidity_timer.ingore_tick();
                if self.humidifier.is_on() {
                    tracing::info!("[SafeMode]: humidifier is on along exhaust, turning off");
                    self.humidifier.turn_off();
                }
                false
            } else {
                humidity_timer.tick()
            };

            if humidifier_on != self.humidifier.is_on() {
                if humidifier_on {
                    tracing::info!("[SafeMode]: Humidifer is on");
                    self.humidifier.turn_on();
                } else {
                    tracing::info!("[SafeMode]: Humidifer is off");
                    self.humidifier.turn_off();
                }
            }

            Timer::after(self.config.loop_delay).await;
        }
    }

    pub async fn display_calibrating(&mut self) {
        if let Some(lcd) = self.lcd.as_mut() {
            let _ = lcd.set_rgb(0, 255, 0).await;
            let _ = lcd.write_str_no("Calibrating").await;
        }
    }

    pub async fn display_measurements(&mut self, stats: Measurements) {
        if let Some(lcd) = self.lcd.as_mut() {
            let mut line: String<32> = String::new();
            let _ = write!(line, "T:{:.1}C H:{:.0}%", stats.temperature, stats.humidity);
            let _ = lcd
                .write_str(
                    0,
                    &line,
                    &WriteSettings::new().align(TextAlign::Center).clear(false),
                )
                .await;
        }
    }

    pub async fn display_fan_state(&mut self, line: String<16>) {
        tracing::info!("[MushclimApp] fan state: {}", line);
        if let Some(lcd) = self.lcd.as_mut() {
            let _ = lcd.clear_row(1).await;
            let _ = lcd
                .write_str(
                    1,
                    &line,
                    &WriteSettings::new().align(TextAlign::Center).clear(false),
                )
                .await;
        }
    }

    /// Flip the LCD into a visual "safe mode" indicator: red backlight + message.
    pub async fn display_safe_mode(&mut self, error_code: u32) {
        if let Some(lcd) = self.lcd.as_mut() {
            let _ = lcd.set_rgb(125, 0, 0).await;

            let msg = match error_code {
                1 => "Err01 Sensor",
                2 => "Err02 Erratic",
                3 => "Err03 NoCalib",
                4 => "Err04 CalibFail",
                5 => "Err05 Exhaust",
                _ => "Err00 Unknown",
            };

            let mut line: String<32> = String::new();
            let _ = write!(line, "{}", msg);
            let _ = lcd.write_str_no(&line).await;
        }
    }
    
    async fn send_stats(&self, measurements: Measurements, exhaust_on: bool, humidifier_on: bool) {
        let stats = MushclimStats {
            temperature: measurements.temperature,
            humidity: measurements.humidity,
            exhaust_on,
            humidifier_on
        };
        
        self.mqtt_handle.publish("mushclim/stats", stats).await;
    }
    
    
    /// Helper to grab measurements and dump them to the logger
    async fn log_current_measurements(&mut self, stats: Measurements, humidifier_on: bool) {
        tracing::info!(
            "[MushclimApp] Temp: {}°C, Humidity: {}% Humidifier: {}",
            stats.temperature,
            stats.humidity,
            humidifier_on
        );
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    loop {
        runner.run().await;
    }
}
