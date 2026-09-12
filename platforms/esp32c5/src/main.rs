#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![feature(allocator_api)]

extern crate alloc;
use core::marker::PhantomData;

use crate::wifi::{WifiCredentials, wifi_task};
use alloc::boxed::Box;
use embassy_executor::Spawner;
use embassy_net::{Runner, StackResources};
use embassy_time::Delay;
use embedded_tls::TlsConfig;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Output, OutputConfig};
use esp_hal::i2c::master::{Config, I2c};
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{Async, time};
use esp_radio::wifi::Interface;
use esp_storage::FlashStorage;
use ivy::mqtt::{MqttModule, MqttState, MqttTcpClient, MqttTcpClientState, MqttTlsState};
use ivy::storage::StorageKey;
use ivy::{count, declare_subcriptions, init_storage, mk_static};
use mushclim::MushclimPlatform;
use mushclim::app::MUSHCLIM_MQTT_PAYLOAD;
use mushclim::app::{MushclimApp, MushclimPins};
use mushclim::config::MushclimConfigDto;
use rand::SeedableRng;
use rand_chacha::ChaChaRng;
use sequential_storage::map::MapConfig;

pub mod wifi;

macro_rules! create_output {
    ($pin:expr) => {
        Output::new($pin, esp_hal::gpio::Level::High, OutputConfig::default())
    };
}

static TCP_BUFFER_SIZE: usize = 9984;

static TLS_BUFFER_SIZE: usize = 16640;

type MushclimMqtt = MqttModule<
    ChaChaRng,
    MUSHCLIM_MQTT_PAYLOAD,
    1,
    TCP_BUFFER_SIZE,
    TCP_BUFFER_SIZE,
    TLS_BUFFER_SIZE,
>;
const CONF_KEY: StorageKey = StorageKey::new(101);
// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    // Allocate Heap for Radio/COEX
    esp_alloc::heap_allocator!(size: 68 * 1024);

    esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram);
    ivy::logger::init_mqtt_logger();
    // esp_println::logger::init_logger(log::LevelFilter::Debug);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
    tracing::info!("Mushclim booted");
    // Init wifi
    let (wifi_controller, interfaces) = esp_radio::wifi::new(peripherals.WIFI, Default::default())
        .expect("Failed to initialize Wi-Fi controller");
    tracing::info!("Wifi started!");

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

    // Declare subs for device
    let (handles, config_sub) = declare_subcriptions! {
        config => "mushclim/config" : MushclimConfigDto
    };

    let config_sub = config_sub.0;

    // NOTE: This is temporary trng workaround for esp32c5, meanwhile trng is not yet available
    let trng = rand_chacha::ChaChaRng::seed_from_u64(seed);

    let mqtt_handle = ivy::actor!(
        spawner,
        MushclimMqtt,
        MushclimMqtt::new(
            "mushdev",
            stack,
            handles,
            leak(MqttState::new()),
            leak(MqttTlsState::new()),
            leak(MqttTcpClient::new(stack, leak(MqttTcpClientState::new()))),
            leak(TlsConfig::default().enable_rsa_signatures()),
            trng
        )
    )
    .unwrap();
    // I2c for sensor
    let i2c = I2c::new(
        peripherals.I2C0,
        Config::default().with_frequency(time::Rate::from_khz(100)),
    )
    .unwrap()
    .with_scl(peripherals.GPIO1)
    .with_sda(peripherals.GPIO0)
    .into_async();

    let pins = MushclimPins::<Esp32c5Platform> {
        light: create_output!(peripherals.GPIO25),
        exhaust: create_output!(peripherals.GPIO7),
        disco: create_output!(peripherals.GPIO23),
        sensor: i2c,
        delay: Delay,
        humidifier: create_output!(peripherals.GPIO24),
    };

    tracing::info!("heapstats {}", esp_alloc::HEAP.stats());

    let mut mushclim_app = MushclimApp::init(pins, storage, mqtt_handle, config_sub).await;

    mushclim_app.run().await
}

struct Esp32c5Platform<'a> {
    life: PhantomData<&'a ()>,
}

impl<'a> MushclimPlatform for Esp32c5Platform<'a> {
    type DiscoPin = Output<'a>;
    type ExhaustPin = Output<'a>;
    type HumidifierPin = Output<'a>;
    type LightPin = Output<'a>;
    type NvsStorage = FlashStorage<'static>;
    type Sensor = I2c<'a, Async>;
    type Delay = Delay;
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    loop {
        runner.run().await;
    }
}

fn leak<T>(value: T) -> &'static mut T {
    Box::leak(Box::new_in(value, esp_alloc::ExternalMemory))
}
