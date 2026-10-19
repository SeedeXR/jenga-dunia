# Security Policy

## Reporting a vulnerability
Please do **not** open a public issue. Use GitHub's private vulnerability reporting on this repository (Security → Report a vulnerability). We aim to acknowledge within 5 business days and to publish a fix or mitigation within 90 days of confirmation.

## Scope
Desktop application, CLI, MCP server, plugin loader, file importers/exporters, downloaders (DEM/OSM/model weights), build and release pipeline.

## Supported versions
Pre-release: `main` only.

## Design
See `docs/architecture/10-security.md` for the threat model, Tauri capability scoping, untrusted-input handling, agent permission scopes and supply-chain controls.
