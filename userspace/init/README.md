# lepton-init

The init process (PID 1) for Hadron OS. Runs in ring 3 as the first userspace process, sets default environment variables (`PATH`, `PWD`, `HOME`), spawns `/bin/sh`, and automatically respawns the shell if it exits.

## Features

- Sets default environment variables for the session (`PATH=/bin`, `PWD=/`, `HOME=/`)
- Spawns the interactive shell (`/bin/sh`) as a child process
- Waits for the shell to exit and reports its exit status
- Respawns the shell in a loop to keep the system interactive
- Minimal `no_std` binary built on top of `lepton-syslib`
