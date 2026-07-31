use alloc::string::String;
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_net::Stack;
use embassy_net::dns::DnsSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::pubsub::WaitResult;
use embedded_mqttc::state::State;
use embedded_mqttc::{ClientConfig, ClientCredentials, Host, QoS};
use esp_println::println;
use static_cell::StaticCell;

const BUFFER: usize = 512;
const TOPIC: usize = 64;
const QUEUE: usize = 4;

type Net = TcpClient<'static, 1, 1024, 1024>;
type MqttState =
    State<'static, 'static, CriticalSectionRawMutex, Net, DnsSocket<'static>, BUFFER, TOPIC, QUEUE>;

static TCP_CLIENT_STATE: StaticCell<TcpClientState<1, 1024, 1024>> = StaticCell::new();
static TCP_CLIENT: StaticCell<Net> = StaticCell::new();
static MQTT_STATE: StaticCell<MqttState> = StaticCell::new();

pub static OUTGOING: Channel<CriticalSectionRawMutex, String, 8> = Channel::new();

// ONE task. No spawning a second task for the runner — run() and the
// app loop live in the same future via select, so nothing needs Send.
#[embassy_executor::task]
pub async fn mqtt_task(stack: Stack<'static>) {
    println!("Starting MQTT task");

    let tcp_client_state = TCP_CLIENT_STATE.init(TcpClientState::new());
    println!("Initializing TCP client");

    let tcp_client = TCP_CLIENT.init(TcpClient::new(stack, tcp_client_state));
    println!("Initialized TCP client");
    let dns_client = DnsSocket::new(stack);
    println!("Connecting to MQTT broker");
    let credentials = ClientCredentials::new("soul", "lsd892kfj3792847ljlDLKJ87DS");
    let config = ClientConfig::new_with_auto_subscribes(
        Host::Hostname("217.195.48.206"),
        Some(8882),
        "mushclim-device",
        Some(credentials),
        ["mushclim.listens"].into_iter(),
        QoS::AtLeastOnce,
    );

    let state = MQTT_STATE.init(State::new(config, None, tcp_client, dns_client));
    let client = state.new_client();
    let mut incoming = state.subscribe_received_publishes().unwrap();

    // no manual subscribe() call — it's handled automatically once run() starts

    let runner = state.run();
    let app_loop = async {
        println!("There");
        loop {
            match select(incoming.next_message(), OUTGOING.receive()).await {
                Either::First(WaitResult::Message(msg)) => {
                    println!("mqtt message received");
                }
                Either::First(_) => {}
                Either::Second(text) => {
                    let _ = client
                        .publish("mushclim.logs", text.as_bytes(), QoS::AtLeastOnce, false)
                        .await;
                }
            }
        }
    };

    select(runner, app_loop).await;
}
