# Use official Rust image to build the project
FROM rust:latest AS builder

# Create a new directory for the application
WORKDIR /usr/src/offline-eth-toolkit

# Copy the Cargo.toml and Cargo.lock to cache dependencies first
COPY Cargo.toml Cargo.lock ./

# Copy the entire source code directory (this includes all binaries)
COPY . .

# Build the project inside the container
RUN cargo build --release

# Final image stage (use a minimal image for the final result)
FROM ubuntu:22.04

# Copy the compiled binaries from the builder stage
COPY --from=builder /usr/src/offline-eth-toolkit/target/release/tx_builder /usr/local/bin/
COPY --from=builder /usr/src/offline-eth-toolkit/target/release/tx_signer /usr/local/bin/
COPY --from=builder /usr/src/offline-eth-toolkit/target/release/tx_inspector /usr/local/bin/
COPY --from=builder /usr/src/offline-eth-toolkit/target/release/tx_broadcaster /usr/local/bin/

# Set the default command to run the main binary (adjust if necessary)
CMD ["offline-eth-toolkit"]
