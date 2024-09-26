Rewrite of afl-fuzz in Rust.

# TODO
- [x] AFL_HANG_TMOUT
- [x] AFL_NO_AUTODICT
- [x] AFL_MAP_SIZE
- [x] AFL_KILL_SIGNAL
- [x] AFL_BENCH_JUST_ONE
- [x] AFL_DEBUG_CHILD
- [x] AFL_PERSISTENT
- [x] AFL_IGNORE_TIMEOUTS
- [x] AFL_EXIT_ON_SEED_ISSUES
- [x] AFL_BENCH_UNTIL_CRASH
- [x] AFL_TMPDIR
- [x] AFL_CRASH_EXITCODE
- [x] AFL_TARGET_ENV
- [x] AFL_IGNORE_SEED_PROBLEMS (renamed to AFL_IGNORE_SEED_ISSUES)
- [x] AFL_CRASH_EXITCODE
- [x] AFL_INPUT_LEN_MIN
- [x] AFL_INPUT_LEN_MAX
- [x] AFL_CYCLE_SCHEDULES
- [x] AFL_CMPLOG_ONLY_NEW
- [x] AFL_PRELOAD
- [x] AFL_SKIP_BIN_CHECK
- [x] AFL_NO_STARTUP_CALIBRATION (this is default in libafl, not sure if this needs to be changed?)
- [x] AFL_FUZZER_STATS_UPDATE_INTERVAL
- [x] AFL_DEFER_FORKSRV
- [x] AFL_NO_WARN_INSTABILITY (we don't warn anyways, we should maybe?)
- [x] AFL_IMPORT_FIRST (implicit)
- [x] AFL_SYNC_TIME 
- [x] AFL_AUTORESUME
- [x] AFL_PERSISTENT_RECORD
- [ ] AFL_FINAL_SYNC 
- [ ] AFL_CRASHING_SEEDS_AS_NEW_CRASH
- [ ] AFL_IGNORE_UNKNOWN_ENVS
- [ ] AFL_NO_UI
- [ ] AFL_PIZZA_MODE :)
- [ ] AFL_EXIT_WHEN_DONE
- [ ] AFL_EXIT_ON_TIME
- [ ] AFL_NO_AFFINITY
- [ ] AFL_FORKSERVER_KILL_SIGNAL
- [ ] AFL_EXPAND_HAVOC_NOW
- [ ] AFL_NO_FORKSRV
- [ ] AFL_FORKSRV_INIT_TMOUT
- [ ] AFL_TRY_AFFINITY
- [ ] AFL_FAST_CAL
- [ ] AFL_NO_CRASH_README
- [ ] AFL_KEEP_TIMEOUTS
- [ ] AFL_TESTCACHE_SIZE
- [ ] AFL_NO_ARITH
- [ ] AFL_DISABLE_TRIM
- [ ] AFL_MAX_DET_EXTRAS
- [ ] AFL_IGNORE_PROBLEMS
- [ ] AFL_IGNORE_PROBLEMS_COVERAGE
- [ ] AFL_STATSD_TAGS_FLAVOR
- [ ] AFL_STATSD
- [ ] AFL_STATSD_PORT
- [ ] AFL_STATSD_HOST
- [ ] AFL_IMPORT
- [ ] AFL_SHUFFLE_QUEUE
- [ ] AFL_CUSTOM_QEMU_BIN
- [ ] AFL_PATH
- [ ] AFL_CUSTOM_MUTATOR_LIBRARY
- [ ] AFL_CUSTOM_MUTATOR_ONLY
- [ ] AFL_PYTHON_MODULE
- [ ] AFL_DEBUG
- [ ] AFL_I_DONT_CARE_ABOUT_MISSING_CRASHES
- [ ] AFL_DUMB_FORKSRV
- [ ] AFL_EARLY_FORKSERVER
- [ ] AFL_NO_SNAPSHOT