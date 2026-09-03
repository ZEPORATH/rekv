
## 1. What is rekv?

`rekv` is a fast and lightweight configuration tool.
Think of it like a small storage box for your app settings:
- It keeps your settings in memory (very fast).
- It can save your settings to a JSON file on disk.
- You can **Read** (`get`), **Change** (`set`), and **Watch** (`watch`) settings in real time.
- Programs can talk to it using a local file (Unix Domain Socket) or network (gRPC).

## 2. Quick Setup and Build

Before you run commands, build the project:

### Build for development
```bash
cargo build
```

### Build for fast production
```bash
cargo build --release
```

After building, you can run the binary directly:
```bash
./target/release/rekv --help
```
Or you can use `cargo run --`:
```bash
cargo run -- --help
```

## 3. How to Start the Server (`daemon`)

`rekv` runs as a background service (called a **daemon**). Other programs or commands connect to this daemon.

### 3.1 Start with default settings
```bash
rekv daemon
```
*(Or just type `rekv` without any subcommand. It will start the daemon automatically).*

Defaults used:
- Config file: `/etc/rekv/config.json` (or empty store if file is not there)
- gRPC Port: `50051`
- Unix Socket: `/var/run/rekv.sock`

### 3.2 Start with a custom JSON config file
Use `--config` or `-c`:
```bash
rekv daemon --config ./tests/fixtures/settings.json
```
or:
```bash
rekv daemon -c ./tests/fixtures/settings.json
```

### 3.3 Start with a custom gRPC port
Use `--port` or `-p`:
```bash
rekv daemon --port 60060
```
or:
```bash
rekv daemon -p 60060
```

### 3.4 Start with a custom Unix Socket file
Use `--uds` or `-u`:
```bash
rekv daemon --uds /tmp/rekv.sock
```
or:
```bash
rekv daemon -u /tmp/rekv.sock
```

### 3.5 Combine all daemon options
You can combine all options in one command:
```bash
rekv daemon -c ./tests/fixtures/settings.json -p 50051 -u /tmp/rekv.sock
```

## 4. How to Read Settings (`get`)

Use the `get` command to read values from `rekv`.

### 4.1 Read a single setting
```bash
rekv get /platform_manager/grpc_port
```
Output example:
```text
50051
```

### 4.2 Read a whole group / subtree
When a path has children, `rekv` gives you the full JSON object:
```bash
rekv get /platform_manager/constants
```
Output example:
```json
{
  "active_area": 1.0,
  "active_thickness": 0.1,
  "pyro_coefficient": 4.5,
  "window_size": 10
}
```

### 4.3 Read everything (Root)
To see all settings in the store:
```bash
rekv get /
```

### 4.4 Read an item from a list (Array index)
You can read array items by their number (starting from 0):

Using slash:
```bash
rekv get /platform_manager/peripherals/0/pin
```
Using brackets:
```bash
rekv get /platform_manager/peripherals[0]/pin
```

### 4.5 Read an item using ID shorthand (`#ID`)
If an item inside a list has an `"id"` field, you can use `#ID` directly:
```bash
rekv get /platform_manager/peripherals#REED_UP/pin
```
Output example:
```text
23
```

You can also read the entire object for that ID:
```bash
rekv get /platform_manager/peripherals#REED_UP
```

### 4.6 Read items with filters (Predicates)
You can filter items inside arrays:

#### Filter by text:
Find the `id` of any item where `type` is `"reed"`:
```bash
rekv get '/platform_manager/peripherals[type="reed"]/id'
```

#### Filter by number comparison:
Find items where `pin` is 22 or bigger:
```bash
rekv get '/platform_manager/peripherals[pin>=22]/id'
```

You can use all these comparison operators:
| Operator | Meaning | Example |
| :--- | :--- | :--- |
| `=` or `==` | Equal | `[type="reed"]` |
| `!=` | Not equal | `[type!="reed"]` |
| `>` | Greater than | `[pin>20]` |
| `>=` | Greater than or equal | `[pin>=22]` |
| `<` | Less than | `[threshold<90]` |
| `<=` | Less than or equal | `[threshold<=85.0]` |

#### Combine multiple filters:
Find items where `type` is `"relay"` AND `default` is `0`:
```bash
rekv get '/platform_manager/peripherals[type="relay"][default=0]/id'
```

### 4.7 Read items using Wildcards (`*` and `**`)
- Single level wildcard `*`:
```bash
rekv get '/platform_manager/io_devices/*/id'
```
- Multi-level recursive wildcard `**`:
```bash
rekv get '/**/pin'
```

### 4.8 Connect to a specific server address or socket
By default, the CLI first checks the local Unix socket, then falls back to `127.0.0.1:50051`.
You can tell it where to connect manually:

Connect to remote machine with `--address` or `-a`:
```bash
rekv -a 192.168.1.100:50051 get /platform_manager/grpc_port
```

Connect to custom socket with `--uds` or `-u`:
```bash
rekv -u /tmp/rekv.sock get /platform_manager/grpc_port
```

## 5. How to Change Settings (`set`)

Use the `set` command to update or create settings.

### 5.1 Set a text (String) value
Always put quotes inside single quotes so the shell sends JSON:
```bash
rekv set /platform_manager/log/level '"debug"'
```
Output:
```text
OK (1 path(s) updated)
  -> /platform_manager/log/level
```

### 5.2 Set a number
```bash
rekv set /platform_manager/grpc_port 50055
```

### 5.3 Set a true / false (Boolean) value
```bash
rekv set /app/active true
```

### 5.4 Set a full JSON object or list
```bash
rekv set /platform_manager/constants '{"active_area": 2.5, "window_size": 20}'
```

### 5.5 Create a brand new setting path
If the path does not exist yet, `rekv` creates it automatically:
```bash
rekv set /my_system/wifi/ssid '"Office_WiFi"'
```

### 5.6 Broadcast update (Change many items at once!)
You can use filters with `set`. `rekv` will find all matching items and update all of them in one command:
```bash
rekv set '/platform_manager/peripherals[type="relay"]/default' 1
```
Output:
```text
OK (3 path(s) updated)
  -> /platform_manager/peripherals/2/default
  -> /platform_manager/peripherals/3/default
  -> /platform_manager/peripherals/4/default
```

### 5.7 Connect to a specific server address or socket for `set`
```bash
rekv -a 127.0.0.1:50051 set /platform_manager/log/level '"info"'
```
or with socket:
```bash
rekv -u /tmp/rekv.sock set /platform_manager/log/level '"info"'
```

## 6. How to Watch Settings Live (`watch`)

Use the `watch` command to listen for changes in real time.
Whenever someone updates a value, you will see a message on your screen immediately.

### 6.1 Watch a specific setting
```bash
rekv watch /platform_manager/log/level
```

### 6.2 Watch a parent folder (Bubble-Up feature)
In `rekv`, changes "bubble up" to parents:
If you watch `/platform_manager`, you will receive notifications for:
- `/platform_manager`
- `/platform_manager/log`
- `/platform_manager/log/level`
- any other change inside `/platform_manager`

```bash
rekv watch /platform_manager
```

### 6.3 Watch everything
```bash
rekv watch /
```

### 6.4 What the watch output looks like
```text
Watching for changes on '/platform_manager' via http://127.0.0.1:50051...
Streaming live events (press Ctrl+C to exit):
[1725345678901] /platform_manager/log/level -> "debug" (was: "info")
```
To stop watching, press `Ctrl + C`.

### 6.5 Watch on a custom server address
```bash
rekv -a 192.168.1.50:50051 watch /platform_manager
```

## 7. Complete Command Summary Table

| Command | What it does | Example |
| :--- | :--- | :--- |
| `rekv daemon` | Starts the background server | `rekv daemon -c config.json -p 50051 -u /tmp/rekv.sock` |
| `rekv get <path>` | Gets a value or subtree | `rekv get /platform_manager/grpc_port` |
| `rekv set <path> <val>` | Sets or updates a value | `rekv set /platform_manager/log/level '"debug"'` |
| `rekv watch <path>` | Streams live updates | `rekv watch /platform_manager` |
| `rekv --help` | Shows general help screen | `rekv --help` |
| `rekv <subcommand> --help` | Shows help for a subcommand | `rekv get --help` |

### Global Flags (can be used with any command)
| Flag | Short | What it means | Default |
| :--- | :--- | :--- | :--- |
| `--address <ADDR>` | `-a` | Remote gRPC server address | `127.0.0.1:50051` |
| `--uds <PATH>` | `-u` | Path to Unix Domain Socket | `/var/run/rekv.sock` |
| `--help` | `-h` | Print help information | - |
| `--version` | `-V` | Print version number | - |

## 8. Simple Step-by-Step Tutorial

Try these 4 steps in your terminal to see everything working together!

### Step 1: Start the server (Terminal 1)
```bash
cargo run -- daemon -c ./tests/fixtures/settings.json -p 50051 -u /tmp/rekv.sock
```
Leave Terminal 1 running.

### Step 2: Open Terminal 2 and start watching
```bash
cargo run -- -a 127.0.0.1:50051 watch /platform_manager
```
Leave Terminal 2 open. It will wait for changes.

### Step 3: Open Terminal 3 and read a value
```bash
cargo run -- -u /tmp/rekv.sock get /platform_manager/log/level
```
You will see:
```text
info
```

### Step 4: Change the value (Terminal 3)
```bash
cargo run -- -u /tmp/rekv.sock set /platform_manager/log/level '"debug"'
```
Now look at Terminal 2! You will see the event appear live:
```text
[...] /platform_manager/log/level -> "debug" (was: "info")
```

## 9. Common Problems and Solutions

- **Problem:** `Connection refused` or `No such file or directory (/var/run/rekv.sock)`
  - **Solution:** The `rekv daemon` is not running. Start it first using `rekv daemon -u /tmp/rekv.sock`, and make sure your client commands use the same socket `-u /tmp/rekv.sock` or address `-a 127.0.0.1:50051`.

- **Problem:** `Path not found: /xyz`
  - **Solution:** The path does not exist in the loaded JSON file. Check your path spelling or use `rekv get /` to see all available paths.

- **Problem:** `Invalid JSON` when using `set`
  - **Solution:** When setting text, remember to wrap it with quotes: `'"my_text"'`. For numbers and booleans, do not use quotes: `123` or `true`.
