# Tests Drive

## Tests unitaires purs

```
cargo test --lib
```

Couvre `validate_name`, `parse_range`, `classify_media`, `resolve_mime`, les enums `NodeKind` / `MediaType`, la validation des secrets et le confinement du stockage. Aucune dépendance externe.

## Tests d'intégration Postgres (US-06)

Les tests d'intégration provisionnent une base Postgres **jetable** : ils créent une base nommée `drive_it_<uuid>`, y appliquent les migrations, exécutent les scénarios, puis détruisent la base en fin de run. Aucune donnée de dev n'est touchée.

### Prérequis

- **Docker Desktop doit tourner.** La base Drive de dev (port 5432) est le conteneur Docker `chabedev-postgres`. Le PostgreSQL 17 natif sur le port 5433 est un piège : ne pas le viser.
- Variable d'environnement `DRIVE_TEST_DATABASE_URL` pointant vers une instance Postgres sur laquelle le rôle a le droit `CREATEDB`. L'URL doit cibler une base de connexion d'administration (souvent `postgres`) ; la base jetable est créée à côté, pas dedans.

Si `DRIVE_TEST_DATABASE_URL` est absente, les tests d'intégration s'auto-ignorent (sortie propre, pas d'échec), afin de garder `cargo test` vert sans Docker.

### Lancement

PowerShell :

```powershell
$env:DRIVE_TEST_DATABASE_URL = "postgres://postgres:postgres@localhost:5432/postgres"
cargo test --test us06_integration_postgres
```

Bash :

```bash
DRIVE_TEST_DATABASE_URL="postgres://postgres:postgres@localhost:5432/postgres" \
  cargo test --test us06_integration_postgres
```

### Conteneur éphémère dédié (recommandé, sans toucher la dev)

Pour une isolation totale du conteneur de dev `chabedev-postgres`, lancer un conteneur éphémère :

```bash
docker run --rm -d --name drive-it-pg -e POSTGRES_PASSWORD=postgres -p 55432:5432 postgres:17
export DRIVE_TEST_DATABASE_URL="postgres://postgres:postgres@localhost:55432/postgres"
cargo test --test us06_integration_postgres
docker stop drive-it-pg
```

### Scénarios couverts

- AC1 — base jetable migrée (tables `nodes` / `drive_users` présentes).
- AC2 — quota respecté transactionnellement (`quota_used_for_update` + rollback, `used_bytes` cohérent).
- AC3 — purge récursive d'une arborescence (nœuds + quotas).
- AC4 — cycle de vie des blobs sans orphelin (purge corbeille sélective).
