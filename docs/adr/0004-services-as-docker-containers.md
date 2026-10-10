# ADR-0004: Start `services:` as Docker containers

Status: Accepted.
Date: 2026-10-10.

## Context

Some test jobs need a database or a cache, for example PostgreSQL or Redis.
They declare it in `services:`. Without the service, the tests fail.

## Decision

Before the first step of the job, localci starts each service with
`docker run -d`. It gives the image, `env`, `ports` and `options` to Docker.
It waits until the health check of the container is `healthy`, for a maximum of
180 seconds. `job.services.<name>.ports.<port>` gives the host port. When the
job stops, localci removes the containers. It also removes a container that did
not start.

The `LOCALCI_DOCKER` variable selects a different command, for example `podman`.
The `--no-services` option starts no containers.

## Consequences

- A service with a fixed host port fails if a local process uses that port.
  The error message tells the user to stop that process or to use
  `--no-services`.
- localci does not create a Docker network. The steps connect to the services
  through the host ports, the same as a job that is not in a container.
