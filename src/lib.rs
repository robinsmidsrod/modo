use std::{sync::Arc, thread, time::Duration};

pub use self::error::{Error, Result};

use chrono::{SubsecRound, Utc};
use clap::Parser;
use rumqttc::{Client, Event, LastWill, MqttOptions, Outgoing, Packet, QoS};
use user_idle::UserIdle;
use wild::ArgsOs;

mod error;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// MQTT server to connect to
    ///
    /// Format:
    /// - Unencrypted: mqtt://myuser:mypassword@example.com:1883?client_id=modo
    /// - Encrypted:  mqtts://myuser:mypassword@example.com:8883?client_id=modo
    #[arg()]
    mqtt_url: String,
    #[arg(short('A'), long, default_value_t = 30)]
    threshold_active: u64,
    #[arg(short('I'), long, default_value_t=5*60)]
    threshold_idle: u64,
    #[arg(short('r'), long, default_value = "modo")]
    mqtt_root_topic: String,
}

pub fn run(args: ArgsOs) -> Result<()> {
    let args = Args::parse_from(args);
    println!("{args:?}");
    let hostname = hostname::get()?.into_string()?.to_ascii_lowercase();
    let topic = format!("{}/{}", &args.mqtt_root_topic, hostname);
    println!("MQTT base topic: {topic}");
    let mut mqtt_options = MqttOptions::parse_url(args.mqtt_url)?;
    mqtt_options.set_last_will(LastWill::new(
        format!("{}/{}/connected", &args.mqtt_root_topic, hostname),
        "false",
        QoS::AtLeastOnce,
        true,
    ));
    let (mqtt_client, mut mqtt_connection) = Client::new(mqtt_options, 10);
    let mqtt_client = Arc::new(mqtt_client);
    let mqtt_client_main = mqtt_client.clone();
    let topic_main = topic.clone();
    thread::spawn(move || {
        let mut previous_published_idle_sec = u64::MAX - 1;
        loop {
            thread::sleep(Duration::from_secs(1));
            let idle = UserIdle::get_time();
            // Print error if any and try again later
            let Ok(idle) = idle else {
                eprintln!("error={:?}", idle.err());
                continue;
            };
            let idle_sec = idle.as_seconds();
            // Publish idle_seconds
            if let Err(e) = mqtt_client.publish(
                format!("{topic}/idle_seconds"),
                QoS::AtLeastOnce,
                true,
                idle_sec.to_string(),
            ) {
                eprintln!("mqtt_publish_idle_seconds_error={e}");
            }
            // Publish idle_status
            let idle_status = match idle_sec {
                i if i < args.threshold_active => "active",
                i if i < args.threshold_idle => "idle",
                _ => "away",
            };
            if let Err(e) = mqtt_client.publish(
                format!("{topic}/idle_status"),
                QoS::AtLeastOnce,
                true,
                idle_status,
            ) {
                eprintln!("mqtt_publish_idle_status_error={e}");
            }
            // If idle_sec is increasing, don't publish
            if idle_sec > previous_published_idle_sec {
                continue;
            }
            // Publish last active timestamp if modified
            let now = Utc::now().trunc_subsecs(0);
            let idle_ts = now - Duration::from_secs(idle_sec);
            if let Err(e) = mqtt_client.publish(
                format!("{topic}/last_active_timestamp"),
                QoS::AtLeastOnce,
                true,
                idle_ts.to_rfc3339(),
            ) {
                eprintln!("mqtt_publish_last_active_timestamp_error={e}");
            }
            previous_published_idle_sec = idle_sec;
        }
    });

    // Poll the MQTT event loop to maintain state
    for notification in mqtt_connection.iter() {
        match notification {
            Ok(notification) => match notification {
                Event::Incoming(p) => match p {
                    Packet::ConnAck(c) => {
                        println!(
                            "MQTT connection status: {:?}, session present: {}",
                            c.code, c.session_present
                        );
                        // Published connected status
                        if let Err(e) = mqtt_client_main.publish(
                            format!("{topic_main}/connected"),
                            QoS::AtLeastOnce,
                            true,
                            "true",
                        ) {
                            eprintln!("mqtt_publish_connected_error={e}");
                        }
                    }
                    Packet::PubAck(_) => {}
                    Packet::PingResp => {}
                    p => {
                        println!("recv={:?}", p);
                    }
                },
                Event::Outgoing(o) => match o {
                    Outgoing::Publish(_) => {}
                    Outgoing::PingReq => {}
                    o => {
                        println!("send={:?}", o);
                    }
                },
            },
            Err(e) => {
                eprintln!("mqtt connection error={e}");
                thread::sleep(Duration::from_secs(10));
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
