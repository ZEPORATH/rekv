TODO
====

[MVP]
- [ ] Finish Rust client: GET/SET/DELETE/LIST
- [ ] Finish Go client: GET/SET/DELETE/LIST
- [ ] Finish C++ client: GET/SET/DELETE/LIST
- [ ] Finish React/TypeScript client
- [ ] Validate selector paths across all clients
      /platform_manager/io_devices[id = ECU0]/baud_rate
      /platform_manager/io_devices[idx = 0]/baud_rate
      /platform_manager/io_devices[id = ECU0]/*
- [ ] Raspberry Pi end-to-end demo
- [ ] Cross-language read/write demo
- [ ] Document setup and quick-start

[PERFORMANCE]
- [ ] Investigate restore timing discrepancy (~405 ms vs ~410 us)
- [ ] Profile RPi base load (~342 ms)
- [ ] Profile overlay load (~619 ms)
- [ ] Implement incremental index updates
- [ ] Reduce/eliminate full store clone on SET (~144 ms RPi)
- [ ] Reduce full index rebuild on SET (~173 ms RPi)
- [ ] Measure mutation/write/fsync separately
- [ ] Benchmark SD card vs USB SSD
- [ ] Re-run benchmark after each optimization

[HARDENING]
- [ ] Expand path parser/error handling tests
- [ ] Test invalid selectors/indexes/wildcards
- [ ] Test concurrent reads/writes
- [ ] Test atomic persistence/restore
- [ ] Test large/deep configuration trees

[FUTURE]
- [ ] Schema/validation layer
- [ ] Subscriptions/config change events
- [ ] Backend abstraction (JSON/SQLite/etc.)
- [ ] Binary protocol if profiling justifies it
- [ ] Authentication/authorization/TLS
- [ ] Transactions/versioning
- [ ] Config migrations

[NON-GOALS FOR NOW]
- No protobuf/gRPC
- No premature binary protocol
- No complex storage engine
- No schema compiler
- No cloud/MQTT/LwM2M integration