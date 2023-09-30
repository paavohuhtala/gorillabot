use std::sync::Arc;

use anyhow::Context;
use serenity::{
    model::prelude::{ChannelId, GuildId, MessageId},
    prelude::TypeMapKey,
};
use tokio::sync::{Mutex, MutexGuard};

use crate::types::Subscription;

#[derive(Clone)]
pub struct BotDb(Arc<Mutex<rusqlite::Connection>>);

impl BotDb {
    pub fn new(db_path: &str) -> Self {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        Self(Arc::new(Mutex::new(conn)))
    }

    async fn conn(&self) -> MutexGuard<'_, rusqlite::Connection> {
        self.0.lock().await
    }

    pub async fn migrate(&self) -> anyhow::Result<()> {
        const INIT_SQL: &'static str = include_str!("./init.sql");
        let conn = self.conn().await;
        conn.execute_batch(INIT_SQL)
            .context("Failed to migrate database")?;

        Ok(())
    }

    pub async fn upsert_subscription(&self, sub: Subscription) -> anyhow::Result<()> {
        let conn = self.conn().await;
        let mut stmt = conn.prepare_cached(
            "INSERT INTO subscriptions (guild_id, channel_id, message_id, server_hostname)
            VALUES (?, ?, ?, ?)
            ON CONFLICT (channel_id, server_hostname) DO UPDATE SET server_hostname = ?, message_id = ?",
        )?;
        stmt.execute((
            sub.guild_id.0,
            sub.channel_id.0,
            sub.message_id.0,
            sub.server_hostname.to_string(),
            sub.server_hostname.to_string(),
            sub.message_id.0,
        ))?;
        Ok(())
    }

    pub async fn delete_subscriptions_by_channel_id(
        &self,
        channel_id: ChannelId,
    ) -> anyhow::Result<usize> {
        let conn = self.conn().await;
        let mut stmt = conn.prepare_cached("DELETE FROM subscriptions WHERE channel_id = ?")?;
        let changes = stmt.execute((channel_id.0,))?;
        Ok(changes)
    }

    pub async fn delete_subscription_by_id(&self, id: i64) -> anyhow::Result<usize> {
        let conn = self.conn().await;
        let mut stmt = conn.prepare_cached("DELETE FROM subscriptions WHERE id = ?")?;
        let changes = stmt.execute((id,))?;
        Ok(changes)
    }

    pub async fn get_subscriptions(&self) -> anyhow::Result<Vec<Subscription>> {
        let conn = self.conn().await;
        let mut stmt = conn.prepare_cached(
            "SELECT id, guild_id, channel_id, message_id, server_hostname FROM subscriptions",
        )?;
        let mut rows = stmt.query(())?;
        let mut subs = Vec::new();
        while let Some(row) = rows.next()? {
            let sub = Subscription {
                id: Some(row.get(0)?),
                guild_id: GuildId(row.get(1)?),
                channel_id: ChannelId(row.get(2)?),
                message_id: MessageId(row.get(3)?),
                server_hostname: row.get::<_, String>(4)?.parse()?,
            };
            subs.push(sub);
        }
        Ok(subs)
    }
}

impl TypeMapKey for BotDb {
    type Value = BotDb;
}
