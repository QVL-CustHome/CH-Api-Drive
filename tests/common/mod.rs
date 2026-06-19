use sqlx::postgres::PgPoolOptions;
use sqlx::{Connection, Executor, PgConnection, Pool, Postgres};
use std::time::Duration;
use uuid::Uuid;

pub const ENV_ADMIN_URL: &str = "DRIVE_TEST_DATABASE_URL";

pub struct DisposableDb {
    admin_url: String,
    db_name: String,
    pub pool: Pool<Postgres>,
}

impl DisposableDb {
    pub async fn create() -> Option<Self> {
        let admin_url = std::env::var(ENV_ADMIN_URL).ok()?;
        let db_name = format!("drive_it_{}", Uuid::new_v4().simple());

        let mut admin = PgConnection::connect(&admin_url)
            .await
            .expect("connexion à la base d'administration impossible");
        admin
            .execute(format!("CREATE DATABASE \"{db_name}\"").as_str())
            .await
            .expect("création de la base jetable impossible");
        admin.close().await.ok();

        let db_url = replace_database(&admin_url, &db_name);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&db_url)
            .await
            .expect("connexion à la base jetable impossible");

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations en échec sur la base jetable");

        Some(Self {
            admin_url,
            db_name,
            pool,
        })
    }

    pub async fn destroy(self) {
        let Self {
            admin_url,
            db_name,
            pool,
        } = self;
        pool.close().await;

        if let Ok(mut admin) = PgConnection::connect(&admin_url).await {
            let _ = admin
                .execute(
                    format!(
                        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
                         WHERE datname = '{db_name}' AND pid <> pg_backend_pid()"
                    )
                    .as_str(),
                )
                .await;
            let _ = admin
                .execute(format!("DROP DATABASE IF EXISTS \"{db_name}\"").as_str())
                .await;
            admin.close().await.ok();
        }
    }
}

fn replace_database(url: &str, db_name: &str) -> String {
    match url.rfind('/') {
        Some(idx) => {
            let base = &url[..idx];
            let query = url[idx + 1..]
                .find('?')
                .map(|q| &url[idx + 1 + q..])
                .unwrap_or("");
            format!("{base}/{db_name}{query}")
        }
        None => format!("{url}/{db_name}"),
    }
}

pub async fn seed_drive_user(pool: &Pool<Postgres>, user_id: &str, quota_bytes: i64) -> Uuid {
    let root_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO drive_users (user_id, quota_bytes, used_bytes, root_node_id) \
         VALUES ($1, $2, 0, $3)",
    )
    .bind(user_id)
    .bind(quota_bytes)
    .bind(root_id)
    .execute(pool)
    .await
    .expect("insertion drive_user");

    sqlx::query(
        "INSERT INTO nodes (id, owner_id, parent_id, kind, name) \
         VALUES ($1, $2, NULL, 'folder', 'root')",
    )
    .bind(root_id)
    .bind(user_id)
    .execute(pool)
    .await
    .expect("insertion nœud racine");

    root_id
}
