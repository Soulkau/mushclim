#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

extern crate alloc;
use alloc::string::{String as AString, ToString};
use bt_hci::controller::ExternalController;
use core::fmt::Write;
use driverse::am2301::Am2301;
use driverse::relay::Relay;
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_net::{Runner, StackResources};
use embassy_time::{Duration, Instant, Timer};
use embedded_storage::nor_flash::ReadNorFlash;
use esp_bootloader_esp_idf::partitions::{self, PartitionEntry};
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
use esp_radio::wifi::WifiDevice;
use esp_storage::FlashStorage;
use mushclim::humidifier::Humidifier;
use mushclim::mqtt::{OUTGOING, start_mqtt};
use mushclim::wifi::{WifiCredentials, WifiManager, WifiStorageV2};

use sequential_storage::map::MapConfig;
use talky::device;
use talky::talky_devices::chest::*;
use talky::types::device::*;

use defmt_rtt as _;
use esp_backtrace as _;

use defmt::{Debug2Format, info};
use heapless::String;
use mushclim::exhaust::ExhaustManager;
use mushclim::lcd::{LCD_ADDRESS, Lcd, RGB_ADDRESS, TextAlign, WriteSettings};
use mushclim::measurements::{
    MeasurementError, MeasurementManager, MeasurementProvider, Measurements, MockSensorProvider,
};
use mushclim::{MushclimConfig, mk_static};
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    info!("Panic occurred: {}", info);
    loop {}
}

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

// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Allocate Heap for Radio/COEX
    esp_alloc::heap_allocator!(#[unsafe(link_section = ".dram2_uninit")] size: 65536);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
    defmt::info!("It is running");
    // Wi-Fi and Bluetooth hardware init
    let (wifi_controller, interfaces) = esp_radio::wifi::new(peripherals.WIFI, Default::default())
        .expect("Failed to initialize Wi-Fi controller");

    let transport = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let ble_controller = ExternalController::<BleConnector, 20>::new(transport);

    let wifi_interface = interfaces.station;
    // let storage = mk_storage!(
    //     FlashStorage,
    //     FlashStorage::new(peripherals.FLASH),
    //     MapConfig::new(0x9000..0xF000)
    // );
    // let device_meta = DeviceMetadata::load(&storage, talky::device::DeviceType::MushClimate).await;

    // #[allow(unused)]
    // let bluetooth_handle: BluetoothHandle = spawn_actor!(
    //     spawner,
    //     BluetoothModule<ExternalController<BleConnector<'static>, 20>>,
    //     BluetoothModule::new(ble_controller, device_meta)
    // );
    let mut rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stack
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        embassy_net::Config::dhcpv4(Default::default()),
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    // let mut password = String::new();
    // let _ = password.write_str("yd3ebEwcgnZ4");
    // let mut ssid = String::new();
    // let _ = ssid.write_str("BALTICOM2G63");
    // info!("Ssid: {}, Password: {}", ssid, password);
    // let creds = WifiCredentials::new(ssid, password);

    // let manager = WifiManager::new(spawner.make_send(), wifi_controller).await;
    // manager.connect(creds).await.expect("Error connecting");
    // spawner.must_spawn(net_task(runner));

    /*
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
            defmt::info!("Here");
            let mut lcd = Lcd::new(
                i2c.with_scl(peripherals.GPIO23)
                    .with_sda(peripherals.GPIO22)
                    .into_async(),
                LCD_ADDRESS,
                RGB_ADDRESS,
            );
            defmt::info!("LCD initialized");
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

    let measurement_manager = MeasurementManager::new(MockSensorProvider::new());
    let config = MushclimConfig::default();
    let exhaust = ExhaustManager::new(fan, &config);
    let humidifier = Humidifier::new(create_relay!(peripherals.GPIO7), &config);
    let mut mushclim: MushclimApp<'static, _> = MushclimApp {
        lcd,
        config,
        measurement_manager,
        lights,
        humidifier,
        disco,
        exhaust,
    };

    mushclim.run().await
    /*  CUSTOM APP INITIALIZATION PLACEHOLDER ---
    let app = todo!("Initialize your custom App structure here wrapped in mk_static!");

    let device_id = create_device_id(peripherals.SHA);
    defmt::info!("{device_id:?}");

    storage.save_master_key(&test_signing_key()).await;

    let encore = Encore::new(storage, app, bluetooth_runner, "Chest", device_id).await;

    let master = storage.load_master_key().await;
    defmt::info!("{master:?}");
    defmt::info!("Encore created");

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
}

impl<'a, P: MeasurementProvider> MushclimApp<'a, P> {
    pub async fn run(&mut self) -> ! {
        self.lights.on();
        self.disco.on();

        defmt::info!("Starting sensor calibration");

        self.display_calibrating().await;

        let Ok(_) = self
            .measurement_manager
            .calibrate(&self.config, self.lcd.as_mut())
            .await
        else {
            defmt::error!("Failed to calibrate measurements");
            self.enter_safe_mode(4).await
        };

        loop {
            self.display_fan_state(self.exhaust.format_state()).await;
            match self.acquire_safe_measurement().await {
                Some(stats) => {
                    self.log_current_measurements(stats);
                    self.display_measurements(stats).await;
                    // "проверить влажность, включить полевалку если надо"
                    self.humidifier.tick(&stats);
                }
                None => {
                    defmt::error!(
                        "CRITICAL: Sensor totally failed or reading is permanently erratic!"
                    );

                    self.enter_safe_mode(1).await;
                }
            }
            self.exhaust.tick(self.humidifier.is_on()).await;
            // Sleep for 1 minute before checking everything again
            Timer::after(self.config.loop_delay).await;
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
                    defmt::warn!("Hardware read error on attempt {}. Retrying...", attempt);
                    Timer::after_secs(2).await;
                }
                Err(MeasurementError::ErraticReading) => {
                    // "сверить новое измерение с историей -> err -> повторить замер и сравнить (5 раз)"
                    defmt::warn!("Spike detected on attempt {}. Re-measuring...", attempt);
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
        defmt::error!("Entering safe mode");

        self.humidifier.turn_off();
        self.exhaust.reset();
        self.display_safe_mode(error_code).await;

        let mut cycle_start = Instant::now();

        loop {
            let elapsed_in_cycle = Instant::now().duration_since(cycle_start);

            // wrap the cycle without drift if we overshoot
            if elapsed_in_cycle >= self.config.safe_humidity_duty_interval {
                let overshoot = elapsed_in_cycle.as_ticks()
                    % self.config.safe_humidity_duty_interval.as_ticks();
                cycle_start = Instant::now() - Duration::from_ticks(overshoot);
            }

            let elapsed_in_cycle = Instant::now().duration_since(cycle_start);
            let should_be_on = elapsed_in_cycle < self.config.safe_humidity_duty_cycle;

            if should_be_on != self.humidifier.is_on() {
                if should_be_on {
                    defmt::info!("Humidifer is on");
                    self.humidifier.turn_on();
                } else {
                    defmt::info!("Humidifer is off");
                    self.humidifier.turn_off();
                }
            }

            self.exhaust.tick(self.humidifier.is_on()).await;

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
        defmt::info!("fan state: {}", line);
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

    /// Helper to grab measurements and dump them to the logger
    fn log_current_measurements(&mut self, stats: Measurements) {
        defmt::info!(
            "Current Stats -> Temp: {}°C, Humidity: {}%",
            stats.temperature,
            stats.humidity
        );
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    loop {
        runner.run().await;
    }
}
