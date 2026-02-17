var builder = DistributedApplication.CreateBuilder(args);

// PostgreSQL - isolated on host port 15432, C locale for Synapse
var pgPassword = builder.AddParameter("pg-password");

var postgres = builder.AddPostgres("robrix-postgres", password: pgPassword, port: 15432)
    .WithEnvironment("POSTGRES_INITDB_ARGS",
        "--auth-host=scram-sha-256 --auth-local=scram-sha-256 --encoding=UTF8 --lc-collate=C --lc-ctype=C")
    .WithDataVolume("robrix-postgres-data")
    .WithLifetime(ContainerLifetime.Persistent);

var synapseDb = postgres.AddDatabase("synapse");

// Synapse - element-hq, native sliding sync enabled by default (1.114+)
var synapse = builder.AddContainer("robrix-synapse", "ghcr.io/element-hq/synapse", "latest")
    .WithHttpEndpoint(port: 8008, targetPort: 8008, name: "client-api")
    .WithEnvironment("UID", "0")
    .WithEnvironment("GID", "0")
    .WithVolume("robrix-synapse-data", "/data")
    .WithBindMount("../config/synapse/homeserver.yaml", "/data/homeserver.yaml", isReadOnly: true)
    .WithBindMount("../config/synapse/log.config", "/data/log.config", isReadOnly: true)
    .WithLifetime(ContainerLifetime.Persistent)
    .WaitFor(synapseDb);

// Element Web - mature browser client for bootstrapping rooms
var elementWeb = builder.AddContainer("robrix-element-web", "vectorim/element-web", "latest")
    .WithHttpEndpoint(port: 8088, targetPort: 80, name: "element-ui")
    .WithBindMount("../config/element/config.json", "/app/config.json", isReadOnly: true)
    .WithLifetime(ContainerLifetime.Persistent)
    .WaitFor(synapse);

builder.Build().Run();
