# Security

## Reporting a problem

If you find a security problem in Loose Screws, please report it privately, not in a
public issue:

1. Open the [Security tab](https://github.com/Nusi42/LooseScrews/security) of this
   repository.
2. Choose **Report a vulnerability** and describe what you found.

Only the maintainer sees the report. Please include what is needed to understand and repeat
the problem: the Loose Screws version or the commit, the steps, what you expected and what
happened, and how serious you think it is. Examples: a message that crashes or stalls the game
server, a way to act for another player or to get around the server's checks, a share code or
invite link that does something it should not, or a problem in the browser client.

You will get an answer within a week. Once the problem is fixed, the fix lands on `main` with
a short note, and you are named as the finder if you wish.

## What is covered

Loose Screws is a preview before version 1.0. Fixes go into the `main` branch (and the newest
release, once there are releases); older versions are not patched.

Please test only against a server you run yourself (see the
[developer guide](docs/dev-guide.md)), never against other people's servers or players, and do
not read, change or delete data that is not yours while you look for a problem.

## How Loose Screws is built for security

The game server is authoritative: clients send inputs, the server validates them and runs the
simulation ([docs/architecture/server.md](docs/architecture/server.md),
[docs/architecture/netcode.md](docs/architecture/netcode.md)).
