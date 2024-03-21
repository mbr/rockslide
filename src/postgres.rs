use thiserror::Error;

#[derive(Debug, Error)]
enum Error {
    #[error(transparent)]
    PostgresError(#[from] tokio_postgres::Error),
}

#[derive(Debug)]
struct PostgresDb {
    config: String,
}

impl PostgresDb {
    async fn connect(&self) -> Result<tokio_postgres::Client, Error> {
        let (client, connection) =
            tokio_postgres::connect(&self.config, tokio_postgres::NoTls).await?;

        tokio::spawn(connection);

        Ok(client)
    }
    /// Creates a new, connected instance of the postgres database.
    async fn new<S: Into<String>>(config: S) -> Self {
        Self {
            config: config.into(),
        }
    }

    async fn run_self_check(&self) -> Result<(), Error> {
        let con = self.connect().await?;

        Ok(())
    }
}
