use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error(transparent)]
    PostgresError(#[from] tokio_postgres::Error),
}

#[derive(Debug)]
pub(crate) struct PostgresDb {
    config: String,
}

impl PostgresDb {
    pub(crate) async fn connect(&self) -> Result<PostgresConnection, Error> {
        let (client, connection) =
            tokio_postgres::connect(&self.config, tokio_postgres::NoTls).await?;

        tokio::spawn(connection);

        Ok(PostgresConnection { client })
    }
    /// Creates a new instance of the postgres database.
    pub(crate) fn new<S: Into<String>>(config: S) -> Self {
        Self {
            config: config.into(),
        }
    }
}

pub(crate) struct PostgresConnection {
    client: tokio_postgres::Client,
}

impl PostgresConnection {
    pub(crate) async fn run_self_check(&self) -> Result<bool, Error> {
        let row = self
            .client
            .query_one(
                "SELECT COUNT(*) FROM pg_namespace WHERE nspname = 'rockslide'",
                &[],
            )
            .await?;

        let count: i64 = row.get(0);

        Ok(count > 0)
    }
}
