use crate::{ApiError, ApiResult, waiting::WaitingQueries};
use std::time::{Duration, Instant};
use tokio::sync::{MappedMutexGuard, Mutex, MutexGuard};
use tokio_postgres::{Client, Config, NoTls};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const RETRY_DELAY: Duration = Duration::from_secs(1);

/// One serialized connection. Reconnection never replays a business transaction.
pub struct Database {
    config: Config,
    state: Mutex<ConnectionState>,
}

struct ConnectionState {
    connection: Option<Connection>,
    retry_at: Instant,
}

pub(crate) struct Connection {
    client: Client,
    waiting_queries: Option<WaitingQueries>,
}

impl Connection {
    pub(crate) async fn waiting_transaction(
        &mut self,
    ) -> ApiResult<(tokio_postgres::Transaction<'_>, &WaitingQueries)> {
        // Statement handles belong to this PostgreSQL session. Prepare after migration,
        // and publish the set only after every statement prepared successfully.
        let queries = match &mut self.waiting_queries {
            Some(queries) => queries,
            slot @ None => slot.insert(WaitingQueries::prepare(&self.client).await?),
        };
        Ok((self.client.transaction().await?, queries))
    }
}

impl Database {
    pub(crate) fn connection_config(
        connection_string: &str,
    ) -> Result<Config, tokio_postgres::Error> {
        let mut config: Config = connection_string.parse()?;
        config
            .keepalives(true)
            .keepalives_idle(Duration::from_secs(5))
            .keepalives_interval(Duration::from_secs(1))
            .keepalives_retries(3)
            .tcp_user_timeout(Duration::from_secs(8));
        Ok(config)
    }

    pub fn new(connection_string: &str) -> Result<Self, tokio_postgres::Error> {
        Ok(Self {
            config: Self::connection_config(connection_string)?,
            state: Mutex::new(ConnectionState {
                connection: None,
                retry_at: Instant::now(),
            }),
        })
    }

    pub async fn lock(&self) -> ApiResult<MappedMutexGuard<'_, Client>> {
        Ok(MappedMutexGuard::map(self.connection().await?, |c| {
            &mut c.client
        }))
    }

    pub(crate) async fn connection(&self) -> ApiResult<MappedMutexGuard<'_, Connection>> {
        let mut state = self.state.lock().await;
        if state
            .connection
            .as_ref()
            .is_none_or(|c| c.client.is_closed())
        {
            if Instant::now() < state.retry_at {
                return Err(ApiError::unavailable());
            }
            // Preserve the cooldown even if the requesting HTTP task is cancelled.
            state.retry_at = Instant::now() + CONNECT_TIMEOUT + RETRY_DELAY;
            let result = tokio::time::timeout(CONNECT_TIMEOUT, self.config.connect(NoTls)).await;
            state.retry_at = Instant::now() + RETRY_DELAY;
            let (client, connection) = match result {
                Ok(Ok(connected)) => connected,
                Ok(Err(error)) => {
                    eprintln!("database_reconnect_failed code={:?}", error.code());
                    return Err(ApiError::unavailable());
                }
                Err(_) => {
                    eprintln!("database_reconnect_timeout");
                    return Err(ApiError::unavailable());
                }
            };
            tokio::spawn(async move {
                if let Err(error) = connection.await {
                    eprintln!("database_connection_closed code={:?}", error.code());
                }
            });
            state.connection = Some(Connection {
                client,
                waiting_queries: None,
            });
            state.retry_at = Instant::now();
        }
        Ok(MutexGuard::map(state, |state| {
            state.connection.as_mut().expect("connected client")
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn stalled_connection_is_bounded_and_retries_are_throttled() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let database = Database::new(&format!(
            "host=127.0.0.1 port={} user=synthetic dbname=synthetic sslmode=disable",
            listener.local_addr().unwrap().port()
        ))
        .unwrap();
        let (result, accepted) = tokio::join!(
            tokio::time::timeout(CONNECT_TIMEOUT + Duration::from_secs(2), database.lock()),
            listener.accept()
        );
        let _socket = accepted.unwrap();
        assert_eq!(result.unwrap().err().unwrap().status.as_u16(), 503);
        let retry = tokio::time::timeout(Duration::from_millis(200), database.lock()).await;
        assert_eq!(retry.unwrap().err().unwrap().status.as_u16(), 503);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn cancelled_connection_attempt_retains_the_cooldown() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let database = Arc::new(
            Database::new(&format!(
                "host=127.0.0.1 port={} user=synthetic dbname=synthetic sslmode=disable",
                listener.local_addr().unwrap().port()
            ))
            .unwrap(),
        );
        let connecting = database.clone();
        let task = tokio::spawn(async move { connecting.lock().await.is_err() });
        let _socket = listener.accept().await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let retry = tokio::time::timeout(Duration::from_millis(200), database.lock()).await;
        assert_eq!(retry.unwrap().err().unwrap().status.as_u16(), 503);
    }
}
