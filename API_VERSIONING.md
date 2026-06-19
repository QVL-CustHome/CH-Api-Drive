# Versionnement de l'API — CH-Api-Drive

## Convention

- Les routes **publiques** sont exposées sous le préfixe `/v1` (`API_VERSION_PREFIX`).
- Les routes **opérationnelles** (`/health`) ne sont pas versionnées : ce sont des sondes d'infrastructure, hors contrat applicatif.
- Les routes **internes** (`/internal/...`) ne sont pas versionnées : elles relèvent d'un contrat inter-services privé, géré par déploiement coordonné, pas par négociation de version client. Drive consomme `/internal/users/resolve` de l'Authenticator (header `x-internal-secret`) mais n'expose aucune route interne propre ; si une telle route apparaît, elle reste hors `/v1`.

## Stratégie de transition

Double exposition temporaire :

- Les routes publiques restent accessibles **sans préfixe** (chemins historiques) pour ne pas casser les consommateurs existants.
- Elles sont **simultanément** disponibles sous `/v1/...`.

Les consommateurs migrent vers `/v1`. Les chemins historiques non préfixés seront retirés une fois la migration des consommateurs confirmée (étape ultérieure, hors de cette livraison).

## Routes publiques versionnées

`/me/storage`, `/files` (+ `/files/{id}/content`, `/files/{id}/thumbnail`), `/gallery`, `/search`, `/duplicates`, `/folders`, `/trash` (+ `/trash/purge`), `/nodes/{id}` (+ `/nodes/{id}/trash`, `/nodes/{id}/restore`), `/admin/users` (+ sous-ressources `/admin/users/{id}`, `/admin/users/{id}/recompute`).

## Cohérence inter-services

Aligné sur CH-Api-Authenticator (SCRUM-130) : même préfixe `/v1`, même schéma de double exposition, mêmes exclusions (opérationnelles et internes non versionnées). Le contrat inter-services `INTERNAL_API_SECRET` / `x-internal-secret` reste inchangé.
