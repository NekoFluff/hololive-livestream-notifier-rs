mod commands;
mod cron;
mod data;
mod discord;
mod pubsub;

use axum::Extension;
use axum::{http::StatusCode, routing::post, Router};
use chrono::{DateTime, Datelike, FixedOffset, NaiveDateTime, Timelike, Utc};
use cron::LivestreamScheduler;
use dotenv::dotenv;
use poise::serenity_prelude::{self as serenity};
use quick_xml::de::from_str;
use std::net::SocketAddr;
use std::sync::Arc;
use std::{env::var, time::Duration};
use tokio::sync::Mutex;

// Types used by all command functions
type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

// Custom data passed to all command functions
pub struct Data {}

// TODO LIST
// TODO: Add logging package
// TODO: Host on heroku
// TODO: Add tracing
// TODO: Reorganize code

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    tokio::spawn(start_bot());
    let livestream_scheduler = Arc::new(Mutex::new(LivestreamScheduler::new().await));

    // tracing_subscriber::fmt::init();
    setup_existing_livestream_notifications(&livestream_scheduler).await;
    tokio::spawn(subscribe_to_feeds());

    let app = Router::new()
        .route("/yt-pubsub", post(yt_pubsub_callback))
        .layer(Extension(livestream_scheduler));
    let addr = SocketAddr::from(([127, 0, 0, 1], std::env::var("PORT")?.parse()?));

    // tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

async fn on_error(error: poise::FrameworkError<'_, Data, Error>) {
    match error {
        poise::FrameworkError::Setup { error, .. } => panic!("Failed to start bot: {:?}", error),
        poise::FrameworkError::Command { error, ctx } => {
            println!("Error in command `{}`: {:?}", ctx.command().name, error,);
        }
        error => {
            if let Err(e) = poise::builtins::on_error(error).await {
                println!("Error while handling error: {}", e)
            }
        }
    }
}

async fn subscribe_to_feeds() {
    let feeds = data::Mongo::new().await.get_feeds().await.unwrap();

    for feed in feeds {
        let pubsub_callback_url = std::env::var("PUBSUB_CALLBACK_URL").ok().unwrap();
        let mut pubsub = pubsub::PubSub::new(reqwest::Url::parse(&pubsub_callback_url)?);

        if let Err(e) = pubsub.subscribe(feed.topic_url.clone()).await {
            println!("Error subscribing to {:?}: {:?}", feed.topic_url, e);
            continue;
        }

        println!(
            "Subscribed to {:?} {:?} ({:?})",
            feed.first_name,
            feed.last_name,
            feed.topic_url.as_str()
        );
    }
}

async fn setup_existing_livestream_notifications(
    livestream_scheduler: &Arc<Mutex<LivestreamScheduler>>,
) {
    let mongo = data::Mongo::new().await;
    let livestreams = mongo.get_livestreams().await.unwrap();

    for livestream in livestreams {
        setup_livestream_notifications(livestream_scheduler, livestream)
            .await
            .unwrap();
    }
}

async fn start_bot() {
    let options = poise::FrameworkOptions {
        commands: vec![
            commands::help(),
            commands::ping(),
            // commands::vote(),
            // commands::getvotes(),
        ],
        prefix_options: poise::PrefixFrameworkOptions {
            prefix: Some("~".into()),
            edit_tracker: Some(poise::EditTracker::for_timespan(Duration::from_secs(3600))),
            additional_prefixes: vec![
                poise::Prefix::Literal("hey bot"),
                poise::Prefix::Literal("hey bot,"),
            ],
            ..Default::default()
        },
        on_error: |error| Box::pin(on_error(error)),
        pre_command: |ctx| {
            Box::pin(async move {
                println!("Executing command {}...", ctx.command().qualified_name);
            })
        },
        post_command: |ctx| {
            Box::pin(async move {
                println!("Executed command {}!", ctx.command().qualified_name);
            })
        },
        command_check: Some(|ctx| {
            Box::pin(async move {
                if ctx.author().id == 123456789 {
                    return Ok(false);
                }
                Ok(true)
            })
        }),
        skip_checks_for_owners: false,
        event_handler: |_ctx, event, _framework, _data| {
            Box::pin(async move {
                println!("Got an event in event handler: {:?}", event.name());
                Ok(())
            })
        },
        ..Default::default()
    };

    poise::Framework::builder()
        .token(var("DISCORD_TOKEN").expect("Missing `DISCORD_TOKEN` env var"))
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                println!("Logged in as {}", _ready.user.name);
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(Data {
                    // votes: Mutex::new(HashMap::new()),
                })
            })
        })
        .options(options)
        .intents(serenity::GatewayIntents::non_privileged())
        .run()
        .await
        .expect("Failed to start bot");
}

async fn yt_pubsub_callback(
    Extension(livestream_scheduler): Extension<Arc<Mutex<cron::LivestreamScheduler>>>,
    payload: String,
) -> StatusCode {
    println!("Processing youtube feed: {}", payload);

    let yt_feed = from_str::<pubsub::YoutubeFeed>(&payload).unwrap();

    println!("Parsed feed: {:?}", yt_feed);

    let livestream_url = yt_feed.entry.first().unwrap().link.href.as_str();

    let mongo = data::Mongo::new().await;
    let livestream = mongo.get_livestream(livestream_url).await;
    match livestream {
        Ok(livestream) => {
            let scraper = data::Scraper::new();
            let stream_dt = scraper.get_stream_dt(livestream_url).await.unwrap();

            if stream_dt > Utc::now() {
                println!("Stream already started ({})", livestream_url);
                return StatusCode::OK;
            }

            let stream_ts_ms = stream_dt.timestamp_millis();
            let updated_ts_ms = yt_feed
                .entry
                .first()
                .unwrap()
                .updated
                .unwrap()
                .timestamp_millis();
            println!("Stream start datetime: {:?}", stream_dt);

            match livestream {
                Some(mut livestream) => {
                    let current_stream_ts_ms = livestream.date.timestamp_millis();

                    if current_stream_ts_ms != stream_ts_ms {
                        livestream.title = yt_feed.entry.first().unwrap().title.clone();
                        livestream.author = yt_feed.entry.first().unwrap().author.name.clone();
                        livestream.date = mongodb::bson::DateTime::from_millis(stream_ts_ms);
                        livestream.updated = mongodb::bson::DateTime::from_millis(updated_ts_ms);

                        mongo.upsert_livestream(&livestream).await.unwrap();
                        setup_livestream_notifications(&livestream_scheduler, livestream)
                            .await
                            .unwrap();
                    }
                }
                None => {
                    let livestream = data::models::Livestream {
                        title: yt_feed.entry.first().unwrap().title.clone(),
                        author: yt_feed.entry.first().unwrap().author.name.clone(),
                        url: livestream_url.to_string(),
                        date: mongodb::bson::DateTime::from_millis(stream_ts_ms),
                        updated: mongodb::bson::DateTime::from_millis(updated_ts_ms),
                    };
                    mongo.insert_livestream(&livestream).await.unwrap();
                    setup_livestream_notifications(&livestream_scheduler, livestream)
                        .await
                        .unwrap();
                }
            }
        }
        Err(e) => {
            println!("Error getting livestream: {}", e);
        }
    }

    StatusCode::OK
}

pub async fn setup_livestream_notifications(
    livestream_scheduler: &Arc<Mutex<LivestreamScheduler>>,
    livestream: data::models::Livestream,
) -> Result<(), Box<dyn std::error::Error>> {
    send_will_livestream_message(&livestream).await.unwrap();

    let timestamp = livestream.date.timestamp_millis() / 1000;
    let date = DateTime::<Utc>::from_utc(
        NaiveDateTime::from_timestamp_opt(timestamp, 0).unwrap(),
        Utc,
    );

    let cron_schedule_str = format!(
        "{} {} {} {} {} *",
        date.second(),
        date.minute(),
        date.hour(),
        date.day(),
        date.month()
    );

    livestream_scheduler
        .lock()
        .await
        .schedule_livestream_notification(
            &cron_schedule_str,
            livestream.url.to_string(),
            Box::new(move |_job_uuid, _scheduler| {
                let livestream_copy = livestream.clone();
                Box::pin(async move {
                    send_is_live_message(&livestream_copy).await.unwrap();
                })
            }),
        )
        .await
        .unwrap();

    Ok(())
}

pub async fn send_will_livestream_message(
    livestream: &data::models::Livestream,
) -> Result<(), Box<dyn std::error::Error>> {
    let timestamp = livestream.date.timestamp_millis() / 1000;
    let mst_dt = DateTime::<FixedOffset>::from_utc(
        NaiveDateTime::from_timestamp_opt(timestamp, 0).unwrap(),
        FixedOffset::west_opt(7 * 3600).unwrap(),
    );

    let message = format!(
        "[{}] will livestream on {} - [{}]",
        livestream.author,
        mst_dt.format("%b %e, %l:%M %p MST"),
        livestream.url
    );

    discord::send_message_to_channel("hololive-notifications", &message).await?;

    Ok(())
}

pub async fn send_is_live_message(
    livestream: &data::models::Livestream,
) -> Result<(), Box<dyn std::error::Error>> {
    let message = format!(
        "[{}] Livestream starting! {}",
        livestream.author, livestream.url
    );

    discord::send_message_to_channel("hololive-stream-started", &message).await?;

    Ok(())
}
