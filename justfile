dev:
    # Sets password to "p"
    FILEBIN_ARGON='$argon2id$v=19$m=4096,t=3,p=1$ZmlsZWJpbl8$UaB9tFyhQqfqNfJi8SECZMooJY80aUOTJXNdWPAkbLc' \
    RUST_LOG=info \
    watchexec --watch templates --watch src --restart cargo run

prof:
    cargo build
    FILEBIN_ARGON='$argon2id$v=19$m=4096,t=3,p=1$ZmlsZWJpbl8$UaB9tFyhQqfqNfJi8SECZMooJY80aUOTJXNdWPAkbLc' \
    RUST_LOG=info \
    samply record target/debug/filebin
