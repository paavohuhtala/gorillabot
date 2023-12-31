use std::env;
use std::net::ToSocketAddrs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use a2s::info::Info;
use a2s::players::Player;
use a2s::A2SClient;
use anyhow::{self, Context as AnyhowContext};
use chrono::{DateTime, Local};
use dotenv::dotenv;
use serenity::async_trait;
use serenity::framework::standard::macros::{command, group};
use serenity::framework::standard::{Args, CommandResult};
use serenity::framework::StandardFramework;
use serenity::model::prelude::{GuildId, Message};
use serenity::prelude::*;

mod db;
mod types;

use db::BotDb;
use tokio::time::Duration;

use crate::types::Subscription;

#[derive(Debug)]
struct Config {
    discord_token: String,
    poll_interval: Duration,
}

struct Handler {
    is_loop_running: AtomicBool,
    config: Arc<Config>,
    a2s_client: Arc<A2SClient>,
}

impl Handler {
    async fn new(config: Arc<Config>) -> Self {
        let a2s_client = A2SClient::new().await.expect("Failed to create A2S client");

        Self {
            is_loop_running: AtomicBool::new(false),
            config,
            a2s_client: Arc::new(a2s_client),
        }
    }
}

#[group]
#[commands(follow_server, unfollow_server)]
#[allowed_roles("gorilladmin")]
#[owner_privilege(false)]
struct AdminOnly;

#[command]
async fn follow_server(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    log::info!(
        "Received follow_server command in channel {}",
        msg.channel_id
    );

    if args.is_empty() {
        msg.reply(ctx, "Expected server hostname").await?;
        return CommandResult::Ok(());
    }

    let server_hostname = args.trimmed().current().unwrap();

    log::info!("Parsing & resolving server hostname: {}", server_hostname);

    match server_hostname.to_socket_addrs() {
        Ok(mut server_hostnames) => {
            if server_hostnames.next().is_none() {
                log::warn!("Failed to resolve server address: {}", server_hostname);
                msg.reply(ctx, "Failed to resolve server address").await?;
                return CommandResult::Ok(());
            };
        }
        Err(_) => {
            log::warn!("Invalid server hostname: {}", server_hostname);
            msg.reply(ctx, "Invalid server hostname").await?;
            return CommandResult::Ok(());
        }
    };

    let message = msg
        .channel_id
        .send_message(&ctx, |m| {
            m.embed(get_server_status_setter(None, server_hostname))
        })
        .await?;

    let data = ctx.data.read().await;
    let db = data.get::<BotDb>().unwrap().clone();

    db.upsert_subscription(Subscription {
        id: None,
        guild_id: msg.guild_id.unwrap(),
        channel_id: msg.channel_id,
        message_id: message.id,
        server_hostname: server_hostname.to_string(),
    })
    .await?;

    msg.react(ctx, '👍').await?;

    CommandResult::Ok(())
}

#[command]
async fn unfollow_server(ctx: &Context, msg: &Message) -> CommandResult {
    let data = ctx.data.read().await;
    let db = data.get::<BotDb>().unwrap();

    db.delete_subscriptions_by_channel_id(msg.channel_id)
        .await?;

    msg.reply(ctx, "Unsubscribed from server status updates :(")
        .await?;

    CommandResult::Ok(())
}

fn get_server_status_setter(
    info: Option<(Info, Vec<Player>)>,
    address: &str,
) -> impl FnOnce(&mut serenity::builder::CreateEmbed) -> &mut serenity::builder::CreateEmbed + '_ {
    let now: DateTime<Local> = Local::now();
    let updated_at = now.format("%Y-%m-%d %H:%M:%S").to_string();

    move |embed| match info {
        Some((info, players)) => {
            let embed = embed
                .field("Server name", info.name.clone(), false)
                .field("Server address", address, false)
                .field("Map", info.map.clone(), false)
                .field("Player count", info.players, false);

            let embed = if players.is_empty() {
                embed
            } else {
                let players = players
                    .into_iter()
                    .map(|player| player.name)
                    .collect::<Vec<_>>()
                    .join(", ");

                embed.field("Players", players, false)
            };

            embed.field("Updated at", updated_at, false)
        }
        None => embed
            .field("Server name", "Unknown", false)
            .field("Server address", address, false)
            .field("Map", "Unknown", false)
            .field("Player count", "Unknown", false)
            .field("Updated at", updated_at, false),
    }
}

fn is_message_was_removed_error(err: &SerenityError) -> bool {
    match err {
        SerenityError::Http(http_error) => match http_error.as_ref() {
            HttpError::UnsuccessfulRequest(res) => res.error.message == "Unknown Message",
            _ => false,
        },
        _ => false,
    }
}

async fn handle_subscription(
    ctx: &Context,
    db: &BotDb,
    a2s_client: &A2SClient,
    subscription: Subscription,
) -> anyhow::Result<()> {
    let info = a2s_client.info(subscription.server_hostname.as_str()).await;

    let info = match info {
        Ok(info) => {
            log::info!(
                "Got server info for {}: {:?}",
                subscription.server_hostname,
                info
            );

            // If there are any players, get them too
            if info.players > 0 {
                let players = a2s_client
                    .players(subscription.server_hostname.as_str())
                    .await;

                match players {
                    Err(err) => {
                        log::warn!(
                            "Failed to get server players for {}: {:?}",
                            subscription.server_hostname,
                            err
                        );
                        Some((info, Vec::new()))
                    }
                    Ok(players) => {
                        log::info!(
                            "Got server players for {}: {:?}",
                            subscription.server_hostname,
                            players
                        );
                        Some((info, players))
                    }
                }
            } else {
                Some((info, Vec::new()))
            }
        }

        Err(err) => {
            log::warn!(
                "Failed to get server info for {}: {:?}",
                subscription.server_hostname,
                err
            );

            None
        }
    };

    let status_setter = get_server_status_setter(info, subscription.server_hostname.as_ref());

    let update_message_result = subscription
        .channel_id
        .edit_message(&ctx, subscription.message_id, |m| m.embed(status_setter))
        .await;

    match update_message_result {
        Ok(_) => {}
        Err(err) if is_message_was_removed_error(&err) => {
            log::warn!("Failed to update message on channel {} because it was removed, removing subscription", subscription.channel_id);

            db.delete_subscription_by_id(
                subscription
                    .id
                    .expect("Subscription from database should always have an ID"),
            )
            .await
            .unwrap();
        }
        Err(err) => {
            log::error!("Failed to update message: {:?}", err);
        }
    }

    Ok(())
}

#[async_trait]
impl EventHandler for Handler {
    async fn cache_ready(&self, ctx: Context, _guilds: Vec<GuildId>) {
        let ctx: Context = ctx.clone();
        let a2s_client = self.a2s_client.clone();
        let config = self.config.clone();

        let db = {
            let data = ctx.data.read().await;
            data.get::<BotDb>().cloned().unwrap()
        };

        if !self.is_loop_running.load(Ordering::Relaxed) {
            tokio::spawn(async move {
                loop {
                    let subscriptions = db.get_subscriptions().await.unwrap();

                    for subscription in subscriptions {
                        handle_subscription(&ctx, &db, &a2s_client, subscription)
                            .await
                            .unwrap();
                    }

                    tokio::time::sleep(config.poll_interval).await;
                }
            });
        }
    }
}

fn get_config_from_env() -> anyhow::Result<Config> {
    let poll_interval = if let Ok(key) = env::var("GORILLA_POLL_INTERVAL_SECONDS") {
        key.parse::<u64>()
            .context("Failed to parse GORILLA_POLL_INTERVAL_SECONDS env var")?
    } else {
        30
    };

    let token =
        env::var("GORILLA_DISCORD_TOKEN").context("Expected GORILLA_DISCORD_TOKEN env var")?;

    Ok(Config {
        poll_interval: Duration::from_secs(poll_interval),
        discord_token: token,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    pretty_env_logger::init();

    log::info!("Loading gorillabot.db");

    let db = BotDb::new("gorillabot.db");

    log::info!("Migrating database");

    db.migrate().await?;

    log::info!("Creating Discord client");

    let config = get_config_from_env()?;
    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;
    let framework = StandardFramework::new()
        .configure(|c| c.prefix("!"))
        .group(&ADMINONLY_GROUP);

    let mut client = Client::builder(config.discord_token.clone(), intents)
        .event_handler(Handler::new(Arc::new(config)).await)
        .framework(framework)
        .await?;

    client.data.write().await.insert::<BotDb>(db);

    if let Err(why) = client.start().await {
        log::error!("An error occurred while running the client: {:?}", why);
    }

    Ok(())
}
