# Machine configuration

Describes the remote machines used to build, test and benchmark OCW on real
hardware (Windows and macOS ports, spec §11.1; graphics spike on a real GPU,
spec §3.1.1).

| File | In git | Contents |
|---|---|---|
| `machines.example.toml` | yes | Documented template with placeholder values |
| `machines.toml` | **no** | Your machines (copy of the template, filled in) |
| `ssh/` | **no** | Private keys and `known_hosts` for these machines |

Everything in `config/` except this README and `*.example.*` files is ignored by
git (see `.gitignore`), so the real configuration and keys cannot be committed by
accident. Check with:

```bash
git check-ignore -v config/machines.toml config/ssh/ocw_linux_ed25519
```

## Setup

1. Copy the template and fill in your machines:

   ```bash
   cp config/machines.example.toml config/machines.toml
   ```

2. Create one key pair per machine (no passphrase, so automated runs work; the
   key only grants access to the dedicated `ocw` account):

   ```bash
   mkdir -p config/ssh && chmod 700 config/ssh
   ssh-keygen -t ed25519 -N "" -C "ocw-linux-gpu" -f config/ssh/ocw_linux_ed25519
   ```

3. Install the public key (`*.pub`) on the machine, in
   `~/.ssh/authorized_keys` of the `ocw` user (Windows:
   `C:\Users\ocw\.ssh\authorized_keys`, or `administrators_authorized_keys`
   for admin accounts).

4. Record the host key once, so later connections are verified:

   ```bash
   ssh-keyscan -p 22 linux-gpu.example.lan >> config/ssh/known_hosts
   ```

5. Test the login:

   ```bash
   ssh -i config/ssh/ocw_linux_ed25519 -o UserKnownHostsFile=config/ssh/known_hosts \
       -o IdentitiesOnly=yes ocw@linux-gpu.example.lan 'uname -a'
   ```

## Per-OS prerequisites on the machines

- **Linux:** Git, rustup, and the packages from `make deps-system`. For
  benchmarks, a logged-in desktop session (Wayland or X11) with the real GPU
  driver (`vulkaninfo --summary` must show the GPU, not llvmpipe).
- **Windows:** OpenSSH Server, Git, rustup, Visual Studio Build Tools with the
  "Desktop development with C++" workload. GUI runs need a logged-in user;
  they are started as a scheduled task in that session (programs started over
  SSH cannot open windows).
- **macOS:** Remote Login (System Settings → General → Sharing), Xcode Command
  Line Tools, rustup. GUI runs need a logged-in user. Bluetooth access asks for
  permission once on screen.
