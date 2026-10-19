use comandos_extensions::cli::retry_concurrent_read;
use std::cell::Cell;

#[test]
fn a_read_that_races_a_writer_is_retried_until_it_passes() {
    let calls = Cell::new(0);
    let result = retry_concurrent_read(|| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            Err("autoridad DB/modo cambió durante lectura".into())
        } else if calls.get() == 2 {
            Err("modo ocupado durante lectura: lock acquisition failed".into())
        } else {
            Ok(())
        }
    });
    assert_eq!(result, Ok(()));
    assert_eq!(calls.get(), 3);
}

#[test]
fn other_errors_and_a_persistent_race_are_reported() {
    let calls = Cell::new(0);
    let other = retry_concurrent_read(|| {
        calls.set(calls.get() + 1);
        Err("Shared catalog missing or unsupported".into())
    });
    assert_eq!(other, Err("Shared catalog missing or unsupported".to_string()));
    assert_eq!(calls.get(), 1);
    calls.set(0);
    let race = retry_concurrent_read(|| {
        calls.set(calls.get() + 1);
        Err("candado de modo cambió durante lectura".into())
    });
    assert!(race.is_err());
    assert_eq!(calls.get(), 6);
}
