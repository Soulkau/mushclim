#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

extern crate alloc;
use bt_hci::controller::ExternalController;
use core::fmt::Write;
use driverse::am2301::Am2301;
use driverse::relay::Relay;
use embassy_executor::Spawner;

use embassy_net::StackResources;
use embassy_time::Timer;

use encore::bluetooth::BluetoothRunner;
use encore::mk_static;
use encore::storage::Storage;

use esp_hal::clock::CpuClock;

use esp_hal::efuse::Efuse;
use esp_hal::gpio::{Output, OutputConfig};
use esp_hal::i2c::master::{Config, I2c};
use esp_hal::peripherals::SHA;
use esp_hal::rng::Rng;
use esp_hal::sha::{Sha, Sha256};
use esp_hal::timer::timg::TimerGroup;
use esp_println::logger::init_logger;
use esp_radio::ble::controller::BleConnector;
use esp_storage::FlashStorage;

use heapless::String;
use log::info;
use mushclim::MushclimConfig;
use mushclim::exhaust::ExhaustManager;
use mushclim::lcd::{LCD_ADDRESS, Lcd, RGB_ADDRESS};
use mushclim::measurements::{
    MeasurementError, MeasurementManager, MeasurementProvider, Measurements,
};

use talky::core::id::DeviceID;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    info!("Panic occurred: {}", info);
    loop {}
}

macro_rules! create_relay {
    ($pin:expr) => {
        Relay::new(
            Output::new(
                $pin,
                esp_hal::gpio::Level::High, // Change once here
                OutputConfig::default(),    // Change once here
            ),
            driverse::relay::ActiveLevel::Low, // Change once here
            false,
        )
        .unwrap()
    };
}

// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(s: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    init_logger(log::LevelFilter::Info);

    // Allocate Heap for Radio/COEX
    esp_alloc::heap_allocator!(#[unsafe(link_section = ".dram2_uninit")] size: 65536);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
    /*
    // Wi-Fi and Bluetooth hardware init
    let (wifi_controller, interfaces) = esp_radio::wifi::new(peripherals.WIFI, Default::default())
        .expect("Failed to initialize Wi-Fi controller");

    let transport = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let ble_controller = ExternalController::<_, 20>::new(transport);
    let wifi_interface = interfaces.station;

    let mut rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stack
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        embassy_net::Config::dhcpv4(Default::default()),
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    // Storage and Flash setup
    let flash: FlashStorage<'static> = FlashStorage::new(peripherals.FLASH);
    let storage = mk_static!(
        Storage<FlashStorage<'static>>,
        Storage::new(flash, MapConfig::new(0x9000..0xF000))
    );

    let bluetooth_runner = BluetoothRunner::new(
        ble_controller,
        Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]),
    );

    */
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
            log::info!("Here");
            let mut lcd = Lcd::new(
                i2c.with_scl(peripherals.GPIO23)
                    .with_sda(peripherals.GPIO22)
                    .into_async(),
                LCD_ADDRESS,
                RGB_ADDRESS,
            );
            log::info!("LCD initialized");
            match lcd.init().await {
                Ok(()) => Some(lcd),
                Err(_) => None,
            }
        }
        Err(_) => None,
    };
    let disco = create_relay!(peripherals.GPIO5);
    let fan = create_relay!(peripherals.GPIO6);
    let humidity = create_relay!(peripherals.GPIO7);
    let lights = create_relay!(peripherals.GPIO0); //Those all are on by default

    let measurement_manager = MeasurementManager::new(am2301);
    let config = MushclimConfig::default();
    let exhaust = ExhaustManager::new(fan, &config);

    let mut mushclim: MushclimApp<'static, _> = MushclimApp {
        lcd,
        config,
        measurement_manager,
        lights,
        humidity,
        disco,
        exhaust,
    };

    mushclim.run().await

    /*  CUSTOM APP INITIALIZATION PLACEHOLDER ---
    let app = todo!("Initialize your custom App structure here wrapped in mk_static!");

    let device_id = create_device_id(peripherals.SHA);
    log::info!("{device_id:?}");

    storage.save_master_key(&test_signing_key()).await;

    let encore = Encore::new(storage, app, bluetooth_runner, "Chest", device_id).await;

    let master = storage.load_master_key().await;
    log::info!("{master:?}");
    log::info!("Encore created");

    let main_loop = async {
        loop {
            Timer::after(Duration::from_millis(10000)).await;
        }
    };

    join(encore.run(), main_loop).await;
    unreachable!()
    */
}

fn create_device_id(sha: SHA<'static>) -> DeviceID {
    let mac = Efuse::mac_address();
    let mut source_data = &mac[..];

    let mut sha = Sha::new(sha);
    let mut hasher = sha.start::<Sha256>();
    let mut output = [0u8; 32];

    while !source_data.is_empty() {
        source_data = nb::block!(hasher.update(source_data)).unwrap();
    }

    hasher.finish(output.as_mut_slice()).unwrap();
    DeviceID::new(output)
}

struct MushclimApp<'a, P: MeasurementProvider> {
    measurement_manager: MeasurementManager<P>,
    lcd: Option<Lcd>,
    exhaust: ExhaustManager<'a>,
    config: MushclimConfig,
    lights: Relay<Output<'a>>,
    humidity: Relay<Output<'a>>,
    disco: Relay<Output<'a>>,
}

impl<'a, P: MeasurementProvider> MushclimApp<'a, P> {
    pub async fn run(&mut self) -> ! {
        self.lights.on();
        self.disco.on();

        log::info!("Starting sensor calibration");

        self.display_calibrating().await;

        let Ok(_) = self.measurement_manager.calibrate(&self.config).await else {
            log::error!("Failed to calibrate measurements");
            self.enter_safe_mode().await
        };

        loop {
            self.exhaust.tick().await;
            match self.acquire_safe_measurement().await {
                Some(stats) => {
                    self.log_current_measurements(stats);
                    self.display_measurements(stats).await;
                    // "проверить влажность, включить полевалку если надо"
                    let current_humidity = stats.humidity;
                    let low_bound = *self.config.humidity_threshold.start();
                    let comfort_bound = *self.config.humidity_threshold.end();

                    if current_humidity <= low_bound && !self.exhaust.is_turned_on() {
                        if !self.humidity.is_on() {
                            log::info!(
                                "Humidity ({}%) below low bound ({low_bound}%). Turning humidifier ON.",
                                current_humidity
                            );
                            self.humidity.on();
                        }
                    } else if current_humidity >= comfort_bound {
                        if self.humidity.is_on() {
                            log::info!(
                                "Humidity ({}%) reached comfort bound ({comfort_bound}%). Turning humidifier OFF.",
                                current_humidity
                            );
                            self.humidity.off();
                        }
                    }
                }
                None => {
                    log::error!(
                        "CRITICAL: Sensor totally failed or reading is permanently erratic!"
                    );

                    self.enter_safe_mode().await;
                }
            }

            // Sleep for 1 minute before checking everything again
            Timer::after_secs(60).await; //IMPORTANT: CHANGE TO MINUTE FOR PRODUCTION
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
                    log::warn!("Hardware read error on attempt {}. Retrying...", attempt);
                    Timer::after_secs(2).await;
                }
                Err(MeasurementError::ErraticReading) => {
                    // "сверить новое измерение с историей -> err -> повторить замер и сравнить (5 раз)"
                    log::warn!("Spike detected on attempt {}. Re-measuring...", attempt);
                    Timer::after_secs(2).await;
                }
                Err(MeasurementError::NotCalibrated) => {
                    self.enter_safe_mode().await;
                }
            }
        }
        None
    }

    async fn enter_safe_mode(&mut self) -> ! {
        self.humidity.off();
        self.exhaust.reset();
        let mut was_exhaust_active = false;
        self.display_safe_mode().await;
        loop {
            let is_exhaust_active = self.exhaust.tick().await;

            // --- 2. HUMIDITY RECOVERY LOGIC ---
            if is_exhaust_active {
                self.humidity.off();
            } else {
                if was_exhaust_active && !is_exhaust_active {
                    log::info!(
                        "Safe Mode: Exhaust finished. Blasting recovery humidity for {}s",
                        self.config.safe_humidity_duty_cycle
                    );

                    self.humidity.on();
                    Timer::after_secs(self.config.safe_humidity_duty_cycle).await;
                    self.humidity.off();
                }
            }

            was_exhaust_active = is_exhaust_active;

            Timer::after_secs(60).await;
        }
    }

    pub async fn display_calibrating(&mut self) {
        if let Some(lcd) = self.lcd.as_mut() {
            let _ = lcd.set_rgb(255, 0, 0).await;
            let _ = lcd.write_str_no("Calibrating").await;
        }
    }

    pub async fn display_measurements(&mut self, stats: Measurements) {
        if let Some(lcd) = self.lcd.as_mut() {
            let mut line: String<32> = String::new();
            let _ = write!(line, "T:{:.1}C H:{:.0}%", stats.temperature, stats.humidity);
            let _ = lcd.write_str_no(&line).await;
        }
    }

    /// Flip the LCD into a visual "safe mode" indicator: red backlight + message.
    pub async fn display_safe_mode(&mut self) {
        if let Some(lcd) = self.lcd.as_mut() {
            let _ = lcd.set_rgb(255, 0, 0).await;
            let _ = lcd.write_str_no("Safe mode").await;
        }
    }

    /// Helper to grab measurements and dump them to the logger
    fn log_current_measurements(&mut self, stats: Measurements) {
        log::info!(
            "Current Stats -> Temp: {}°C, Humidity: {}%",
            stats.temperature,
            stats.humidity
        );
    }
}
