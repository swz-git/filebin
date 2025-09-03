dev_binary_loc := '''target/debug/filebin'''

dev:
    watchexec --watch templates --watch src --watch .env --restart "cargo build && {{ dev_binary_loc }}"

prof:
    cargo build
    samply record {{ dev_binary_loc }}
