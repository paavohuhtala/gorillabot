use std::env;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use a2s::A2SClient;
use anyhow::{self, Context as AnyhowContext};
use dotenv::dotenv;
use serenity::async_trait;
use serenity::framework::standard::macros::{command, group};
use serenity::framework::standard::{Args, CommandResult};
use serenity::framework::StandardFramework;
use serenity::model::prelude::{ChannelId, GuildId, Message, MessageId};
use serenity::prelude::*;
use sled::Tree;

#[derive(Debug)]
struct Config {
    discord_token: String,
    discord_channel_id: ChannelId,
    discord_message_id: MessageId,

    poll_interval: tokio::time::Duration,
    server_hostname: SocketAddr,
}

#[derive(Debug)]
struct Subscription {
    channel_id: ChannelId,
    message_id: MessageId,
    server_hostname: SocketAddr,
}

struct Handler {
    is_loop_running: AtomicBool,
    config: Arc<Config>,
    arma_client: Arc<A2SClient>,
}

impl Handler {
    fn new(config: Arc<Config>) -> Self {
        let arma_client = A2SClient::new().expect("Failed to create A2S client");

        Self {
            is_loop_running: AtomicBool::new(false),
            config,
            arma_client: Arc::new(arma_client),
        }
    }
}

#[group]
#[commands(follow_server, unfollow_server)]
struct General;

#[command]
async fn follow_server(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    if args.is_empty() {
        msg.reply(ctx, "Expected server hostname").await?;
        return CommandResult::Ok(());
    }

    let server_hostname = args.trimmed().current().unwrap();

    let data = ctx.data.read().await;
    let db = data.get::<BotDb>().unwrap();

    let message = msg
        .channel_id
        .send_message(&ctx, |m| {
            m.embed(|e| {
                e.title("Server status")
                    .field("Server name", "Unknown", false)
                    .field("Server address", server_hostname, false)
                    .field("Map", "Unknown", false)
                    .field("Players", "Unknown", false)
            })
        })
        .await?;

    db.set_channel_subscription(
        msg.guild_id.unwrap(),
        msg.channel_id,
        message.id,
        server_hostname,
    )?;

    msg.react(ctx, '👍').await?;

    CommandResult::Ok(())
}

#[command]
async fn unfollow_server(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let data = ctx.data.read().await;
    let db = data.get::<BotDb>().unwrap();

    db.remove_channel_subscription(msg.guild_id.unwrap())?;

    msg.reply(ctx, "Unsubscribed from server status updates :(")
        .await?;

    CommandResult::Ok(())
}

#[async_trait]
impl EventHandler for Handler {
    async fn cache_ready(&self, ctx: Context, _guilds: Vec<GuildId>) {
        let ctx: Context = ctx.clone();
        let arma_client = self.arma_client.clone();
        let config = self.config.clone();

        let db = {
            let data = ctx.data.read().await;
            data.get::<BotDb>().unwrap().clone()
        };

        if !self.is_loop_running.load(Ordering::Relaxed) {
            tokio::spawn(async move {
                loop {
                    let guilds = db.get_guilds().expect("Failed to get guilds from DB");
                    println!("Guilds: {:?}", guilds);

                    let info = arma_client.info(&config.server_hostname).unwrap();
                    println!("{:#?}", info);

                    if let Err(err) = config
                        .discord_channel_id
                        .edit_message(&ctx, config.discord_message_id, |m| {
                            m.embed(|e| {
                                e.title("Server status")
                                    .field("Server name", info.name, false)
                                    .field("Map", info.map, false)
                                    .field("Players", info.players, false)
                            })
                        })
                        .await
                    {
                        log::error!("Failed to send message: {:?}", err);
                    }

                    tokio::time::sleep(config.poll_interval).await;
                }
            });
        }
    }
}

fn get_config_from_env() -> anyhow::Result<Config> {
    let poll_interval = env::var("GORILLA_ARMA_POLL_INTERVAL_SECONDS")
        .context("Expected GORILLA_ARMA_POLL_INTERVAL_SECONDS env var")?
        .parse::<u64>()
        .context("Invalid poll interval")?;
    let server_hostname = env::var("GORILLA_ARMA_HOSTNAME")
        .context("Expected GORILLA_ARMA_HOSTNAME env var")?
        .to_socket_addrs()?
        .next()
        .context("Invalid server hostname")?;

    let token =
        env::var("GORILLA_DISCORD_TOKEN").context("Expected GORILLA_DISCORD_TOKEN env var")?;

    let discord_channel_id = env::var("GORILLA_DISCORD_CHANNEL_ID")
        .context("Expected GORILLA_DISCORD_CHANNEL_ID env var")?
        .parse::<u64>()
        .context("Invalid channel id")?
        .into();

    let discord_message_id = env::var("GORILLA_DISCORD_MESSAGE_ID")
        .context("Expected GORILLA_DISCORD_MESSAGE_ID env var")?
        .parse::<u64>()
        .context("Invalid message id")?
        .into();

    Ok(Config {
        poll_interval: tokio::time::Duration::from_secs(poll_interval),
        server_hostname,
        discord_token: token,
        discord_channel_id,
        discord_message_id,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv()?;
    pretty_env_logger::init();

    let db = sled::open("gorillabot.sled").expect("Failed to open sled db");

    let config = get_config_from_env()?;

    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;

    let framework = StandardFramework::new()
        .configure(|c| c.prefix("!"))
        .group(&GENERAL_GROUP);

    let mut client = Client::builder(config.discord_token.clone(), intents)
        .event_handler(Handler::new(Arc::new(config)))
        .framework(framework)
        .await?;

    client
        .data
        .write()
        .await
        .insert::<BotDb>(BotDb(Arc::new(db)));

    if let Err(why) = client.start().await {
        println!("An error occurred while running the client: {:?}", why);
    }

    Ok(())
}

#[derive(Clone)]
struct BotDb(Arc<sled::Db>);

impl TypeMapKey for BotDb {
    type Value = BotDb;
}

impl BotDb {
    const CHANNEL_ID_KEY: &'static str = "channel_id";
    const SERVER_HOSTNAME_KEY: &'static str = "server_hostname";
    const MESSAGE_ID_KEY: &'static str = "message_id";

    fn get_guild_tree(&self, guild_id: GuildId) -> anyhow::Result<Tree> {
        let tree = self.0.open_tree(guild_id.to_string()).with_context(|| {
            format!("Failed to get or create DB keyspace for guild id {guild_id}")
        })?;

        Ok(tree)
    }

    fn set_channel_subscription(
        &self,
        guild_id: GuildId,
        channel_id: ChannelId,
        message_id: MessageId,
        server_hostname: &str,
    ) -> anyhow::Result<()> {
        let tree = self.get_guild_tree(guild_id)?;
        let channel_id = channel_id.0.to_string();
        let message_id = message_id.0.to_string();

        tree.insert(Self::CHANNEL_ID_KEY, channel_id.as_str())?;
        tree.insert(Self::SERVER_HOSTNAME_KEY, server_hostname)?;
        tree.insert(Self::MESSAGE_ID_KEY, message_id.as_str())?;

        Ok(())
    }

    fn remove_channel_subscription(&self, guild_id: GuildId) -> anyhow::Result<()> {
        let tree = self.get_guild_tree(guild_id)?;

        tree.remove(Self::CHANNEL_ID_KEY)?;
        tree.remove(Self::SERVER_HOSTNAME_KEY)?;
        tree.remove(Self::MESSAGE_ID_KEY)?;

        Ok(())
    }

    fn get_channel_subscription(&self, guild_id: GuildId) -> anyhow::Result<Option<Subscription>> {
        let tree = self.get_guild_tree(guild_id)?;

        let channel_id = tree.get(Self::CHANNEL_ID_KEY)?.map(|v| {
            String::from_utf8(v.to_vec())
                .expect("Failed to parse channel id from DB")
                .parse::<u64>()
                .expect("Failed to parse channel id from DB")
                .into()
        });

        let message_id = tree.get(Self::MESSAGE_ID_KEY)?.map(|v| {
            String::from_utf8(v.to_vec())
                .expect("Failed to parse message id from DB")
                .parse::<u64>()
                .expect("Failed to parse message id from DB")
                .into()
        });

        let server_hostname = tree.get(Self::SERVER_HOSTNAME_KEY)?.map(|v| {
            String::from_utf8(v.to_vec())
                .expect("Failed to parse server hostname from DB")
                .parse::<SocketAddr>()
                .expect("Failed to parse server hostname from DB")
        });

        Ok(match (channel_id, message_id, server_hostname) {
            (Some(channel_id), Some(message_id), Some(server_hostname)) => Some(Subscription {
                channel_id,
                message_id,
                server_hostname,
            }),
            _ => None,
        })
    }

    fn get_guilds(&self) -> anyhow::Result<Vec<GuildId>> {
        let mut guilds = Vec::new();

        for guild_id in self.0.tree_names() {
            println!("Guild id: {:?}", guild_id);
            let id_str = String::from_utf8(guild_id.to_vec())?;
            println!("Guild id str: {:?}", id_str);
            guilds.push(GuildId(id_str.parse::<u64>()?));
        }

        Ok(guilds)
    }
}
