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
use embassy_futures::select::{Either, select};
use embassy_net::{Runner, StackResources};
use embassy_time::{Ticker, Timer};
use esp_alloc::HeapStats;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Output, OutputConfig};
use esp_hal::i2c::master::{Config, I2c};
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use esp_radio::ble::controller::BleConnector;
use esp_radio::wifi::Interface;
use esp_storage::FlashStorage;
use ivy::mqtt::{MqttHandle, MqttModule, Subscription};
use ivy::storage::{StorageKey, StorageModule};
use ivy::{count, init_storage};
use mushclim::humidifier::Humidifier;
use mushclim::metrics::MetricManager;
use mushclim::timer::CycleTimer;
use sequential_storage::map::MapConfig;

use heapless::String;
use mushclim::exhaust::ExhaustManager;
use mushclim::measurements::{
    MeasurementError, MeasurementManager, MeasurementProvider, Measurements,
};
use mushclim::{MushclimConfig, MushclimConfigDto, MushclimStats, mk_static};

use crate::wifi::{WifiCredentials, wifi_task};

pub mod wifi;

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
    // Allocate Heap for Radio/COEX
    esp_alloc::heap_allocator!(size: 131072);
    // ivy::logger::init_mqtt_logger();
    esp_println::logger::init_logger(log::LevelFilter::Debug);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
    tracing::info!("Mushclim booted");

    let (wifi_controller, interfaces) = esp_radio::wifi::new(peripherals.WIFI, Default::default())
        .expect("Failed to initialize Wi-Fi controller");
    tracing::info!("Wifi started!");
    let stats: HeapStats = esp_alloc::HEAP.stats();
    // HeapStats implements the Display and defmt::Format traits, so you can
    // pretty-print the heap stats.
    println!("{}", stats);
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
    spawner.spawn(wifi_task(wifi_controller, creds).unwrap());

    // Init network stack
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        embassy_net::Config::dhcpv4(Default::default()),
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    spawner.spawn(net_task(runner).unwrap());

    let trng = Rng::new();
    // let (handles, config_sub) = declare_topics! {
    //     config => "mushclim/config" : MushclimConfigDto
    // };
    // let config_sub = config_sub.0;
    // let mqtt_handle =
    //     ivy::actor!(spawner, MqttModule<Rng, 312, 1>, MqttModule::new(stack, trng, handles))
    //         .unwrap();

    let mut out = Output::new(
        peripherals.GPIO4,
        esp_hal::gpio::Level::Low,
        OutputConfig::default().with_drive_mode(esp_hal::gpio::DriveMode::OpenDrain),
    )
    .into_flex();
    out.set_input_enable(true);
    let am2301 = Am2301::new(out);

    let disco = create_relay!(peripherals.GPIO5);
    let fan = create_relay!(peripherals.GPIO6);

    let lights = create_relay!(peripherals.GPIO0); //Those all are on by default

    let measurement_manager = MeasurementManager::new(am2301);
    let config: MushclimConfig = storage
        .get::<MushclimConfigDto>(CONF_KEY)
        .await
        .unwrap_or(MushclimConfigDto::default())
        .as_local();
    let exhaust = ExhaustManager::new(fan, &config);
    let humidifier = Humidifier::new(create_relay!(peripherals.GPIO7), &config);
    let metrics = MetricManager::new(&config);
    // let mut mushclim: MushclimApp<'static, _> = MushclimApp {
    //     lcd,
    //     config,
    //     measurement_manager,
    //     lights,
    //     humidifier,
    //     disco,
    //     exhaust,
    //     metrics,
    //     config_sub,
    //     mqtt_handle,
    //     storage,
    // };
    let stats: HeapStats = esp_alloc::HEAP.stats();
    // HeapStats implements the Display and defmt::Format traits, so you can
    // pretty-print the heap stats.
    println!("{}", stats);
    tracing::info!("Mushclim app was built and running!");
    // mushclim.run().await
    loop {
        Timer::after_millis(5000).await;
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    loop {
        runner.run().await;
    }
}
