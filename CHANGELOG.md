# Changelog

## [0.1.10](https://github.com/Roberdan/convergio-mesh/compare/v0.1.9...v0.1.10) (2026-04-14)


### Features

* sync IPC messages + agents across mesh, support TEXT primary keys ([4b20011](https://github.com/Roberdan/convergio-mesh/commit/4b20011b3b66a8dad5698d800a9979b7201f9bfe))

## [0.1.9](https://github.com/Roberdan/convergio-mesh/compare/v0.1.8...v0.1.9) (2026-04-14)


### Features

* sync IPC messages + agents across mesh, support TEXT primary keys ([4b20011](https://github.com/Roberdan/convergio-mesh/commit/4b20011b3b66a8dad5698d800a9979b7201f9bfe))


### Bug Fixes

* mesh heartbeat uses dev-local token fallback instead of skipping ([eb0c800](https://github.com/Roberdan/convergio-mesh/commit/eb0c80068f605f15b4115845b3c5919ddafe7055))
* **security:** audit pass  HMAC protocol, replay, FK guard, URL encoding2 ([5393cad](https://github.com/Roberdan/convergio-mesh/commit/5393cadf6c9c81f02bf3e73143c11dada2790be1))

## [0.1.8](https://github.com/Roberdan/convergio-mesh/compare/v0.1.7...v0.1.8) (2026-04-13)


### Bug Fixes

* pass CARGO_REGISTRY_TOKEN to release workflow ([4f4ed13](https://github.com/Roberdan/convergio-mesh/commit/4f4ed1343225c9916cf853dc82b4a99bbfb68b52))

## [0.1.7](https://github.com/Roberdan/convergio-mesh/compare/v0.1.6...v0.1.7) (2026-04-13)


### Bug Fixes

* add crates.io publishing metadata (description, repository) ([762b1ce](https://github.com/Roberdan/convergio-mesh/commit/762b1ce0c3a1fc4132ba8e5ede9cede1903d8ca4))

## [0.1.6](https://github.com/Roberdan/convergio-mesh/compare/v0.1.5...v0.1.6) (2026-04-13)


### Features

* adapt convergio-mesh for standalone repo ([79c7c09](https://github.com/Roberdan/convergio-mesh/commit/79c7c0997b22248485ba8e16eb78dd458ad81f1a))
* add task_evidence, delegations, solve_sessions, agent_catalog to mesh sync ([349a2c8](https://github.com/Roberdan/convergio-mesh/commit/349a2c8ef82444c36d8a709b27c83d805dcc23e7))
* initial convergio-mesh from template ([5fa434f](https://github.com/Roberdan/convergio-mesh/commit/5fa434f87c482c2b9b35914420f4986b55656c62))


### Bug Fixes

* align SDK dependency to v0.1.9 for type compatibility ([6f70cb8](https://github.com/Roberdan/convergio-mesh/commit/6f70cb8bb897c18bc661d93c4b17343d896ba42b))
* **release:** use vX.Y.Z tag format (remove component) ([8b7d7f8](https://github.com/Roberdan/convergio-mesh/commit/8b7d7f8435a5d15d623e6024e0e4eb4ed9806072))
* **security:** comprehensive audit — SQL injection, SSRF, HMAC, auth hardening ([#7](https://github.com/Roberdan/convergio-mesh/issues/7)) ([c92c6ac](https://github.com/Roberdan/convergio-mesh/commit/c92c6acc94786ddfb9a86eb21a0b50b5c7819ba5))


### Documentation

* add .env.example with required environment variables ([#9](https://github.com/Roberdan/convergio-mesh/issues/9)) ([d1b51a7](https://github.com/Roberdan/convergio-mesh/commit/d1b51a73b27d4a53e5dd8515135bd99dcceafb3d))

## [0.1.5](https://github.com/Roberdan/convergio-mesh/compare/convergio-mesh-v0.1.4...convergio-mesh-v0.1.5) (2026-04-12)


### Bug Fixes

* align SDK dependency to v0.1.9 for type compatibility ([6f70cb8](https://github.com/Roberdan/convergio-mesh/commit/6f70cb8bb897c18bc661d93c4b17343d896ba42b))

## [0.1.4](https://github.com/Roberdan/convergio-mesh/compare/convergio-mesh-v0.1.3...convergio-mesh-v0.1.4) (2026-04-12)


### Documentation

* add .env.example with required environment variables ([#9](https://github.com/Roberdan/convergio-mesh/issues/9)) ([d1b51a7](https://github.com/Roberdan/convergio-mesh/commit/d1b51a73b27d4a53e5dd8515135bd99dcceafb3d))

## [0.1.3](https://github.com/Roberdan/convergio-mesh/compare/convergio-mesh-v0.1.2...convergio-mesh-v0.1.3) (2026-04-12)


### Bug Fixes

* **security:** comprehensive audit — SQL injection, SSRF, HMAC, auth hardening ([#7](https://github.com/Roberdan/convergio-mesh/issues/7)) ([c92c6ac](https://github.com/Roberdan/convergio-mesh/commit/c92c6acc94786ddfb9a86eb21a0b50b5c7819ba5))

## [0.1.2](https://github.com/Roberdan/convergio-mesh/compare/convergio-mesh-v0.1.1...convergio-mesh-v0.1.2) (2026-04-12)


### Features

* adapt convergio-mesh for standalone repo ([79c7c09](https://github.com/Roberdan/convergio-mesh/commit/79c7c0997b22248485ba8e16eb78dd458ad81f1a))
* add task_evidence, delegations, solve_sessions, agent_catalog to mesh sync ([349a2c8](https://github.com/Roberdan/convergio-mesh/commit/349a2c8ef82444c36d8a709b27c83d805dcc23e7))
* initial convergio-mesh from template ([5fa434f](https://github.com/Roberdan/convergio-mesh/commit/5fa434f87c482c2b9b35914420f4986b55656c62))

## [0.1.1](https://github.com/Roberdan/convergio-mesh/compare/convergio-mesh-v0.1.0...convergio-mesh-v0.1.1) (2026-04-11)


### Features

* adapt convergio-mesh for standalone repo ([79c7c09](https://github.com/Roberdan/convergio-mesh/commit/79c7c0997b22248485ba8e16eb78dd458ad81f1a))
* initial convergio-mesh from template ([5fa434f](https://github.com/Roberdan/convergio-mesh/commit/5fa434f87c482c2b9b35914420f4986b55656c62))

## 0.1.0 (Initial Release)

### Features

- Initial extraction from convergio monorepo
