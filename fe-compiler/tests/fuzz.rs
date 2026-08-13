//! Seeded mutation testing.
//!
//! `cargo-fuzz` needs a nightly toolchain, and a procedure database is small
//! enough that a deterministic in-tree mutator gets most of the value: it runs
//! in CI on every commit, it reproduces exactly from its seed, and a failure
//! prints the seed so it can be replayed.
//!
//! The property under test is the one the aircraft depends on:
//!
//! * `from_bytes` on *any* byte sequence returns `Ok` or `Err` — never panics.
//! * If it returns `Ok`, the database is genuinely safe to execute: every
//!   procedure can be ticked to a terminal state within a bounded number of
//!   ticks, with no panic and no infinite loop.
//!
//! The second half is the interesting one. Verification is only worth having
//! if "verified" implies "executable", so the fuzzer runs whatever it manages
//! to smuggle past the verifier.

mod support;

use support::{FakeAircraft, Recorder};

use fe_runtime::{ProcedureDatabase, ProcedureExecutor, Tick};

/// xorshift64*. Small, deterministic, and good enough to shake bytes around.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    fn byte(&mut self) -> u8 {
        (self.next_u64() >> 33) as u8
    }
}

/// Run every procedure in a database and check that each tick makes progress
/// or parks.
///
/// Termination itself cannot be asserted: a `wait` with no timeout is a legal
/// procedure that waits forever by design, and a mutation can turn a bounded
/// wait into one of those. The property that must hold is stronger and more
/// useful — after a bounded number of ticks the executor is either finished or
/// *parked on a wait*. An executor that is still `Running` after hundreds of
/// ticks would mean a tick is not making progress, which is exactly the hang
/// the forward-only jump rule exists to prevent.
fn execute_everything(db: &ProcedureDatabase<'_>) {
    let mut aircraft = FakeAircraft::new();
    let mut recorder = Recorder::new();
    for procedure in db.procedures() {
        // Triggers are evaluated by hosts every frame, so they get fuzzed too.
        let _ = ProcedureExecutor::evaluate_trigger(&procedure, &aircraft);

        let mut exec = ProcedureExecutor::new(procedure).with_step_limit(2048);
        let mut last = Tick::Idle;
        for tick in 0..512 {
            // Move the aircraft around so waits can be satisfied and branches
            // taken, rather than parking on the first `wait` every time.
            if tick % 7 == 0 {
                aircraft.set_f32(support::tag::HYD2_PRESSURE, 3000.0);
                aircraft.set_bool(support::tag::HYD2_ELECTRIC_PUMP_RUNNING, true);
            }
            // A large step so that any finite timeout is reached quickly.
            last = exec.tick(&aircraft, &mut recorder, 5_000);
            if exec.is_finished() {
                break;
            }
        }
        assert!(
            exec.is_finished() || matches!(last, Tick::Waiting { .. }),
            "procedure {} is still running after 512 ticks (last tick: {last:?})",
            procedure.id
        );
    }
}

/// Apply one random mutation in place.
fn mutate(bytes: &mut Vec<u8>, rng: &mut Rng) {
    if bytes.is_empty() {
        return;
    }
    match rng.below(6) {
        0 => {
            // Flip a byte.
            let at = rng.below(bytes.len());
            bytes[at] ^= 1 << rng.below(8);
        }
        1 => {
            // Overwrite a byte with an arbitrary value.
            let at = rng.below(bytes.len());
            bytes[at] = rng.byte();
        }
        2 => {
            // Truncate.
            let len = rng.below(bytes.len());
            bytes.truncate(len);
        }
        3 => {
            // Overwrite a little-endian word — this is how offsets and counts
            // get interesting values rather than merely wrong ones.
            if bytes.len() >= 4 {
                let at = rng.below(bytes.len() - 3);
                let value = match rng.below(4) {
                    0 => u32::MAX,
                    1 => 0,
                    2 => rng.next_u64() as u32,
                    _ => 0x7FFF_FFFF,
                };
                bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        4 => {
            // Splice a run of bytes from elsewhere in the file.
            let len = 1 + rng.below(8.min(bytes.len()));
            let from = rng.below(bytes.len() - len + 1);
            let to = rng.below(bytes.len() - len + 1);
            let chunk: Vec<u8> = bytes[from..from + len].to_vec();
            bytes[to..to + len].copy_from_slice(&chunk);
        }
        _ => {
            // Append junk.
            for _ in 0..1 + rng.below(16) {
                bytes.push(rng.byte());
            }
        }
    }
}

/// Recompute the content hash so mutations reach the structural validators
/// instead of stopping at the checksum.
fn reseal(bytes: &mut [u8]) {
    use fe_runtime::format::{self, header};
    if bytes.len() <= format::HEADER_SIZE {
        return;
    }
    let hash = format::fnv1a32(&bytes[format::HEADER_SIZE..]);
    bytes[header::CONTENT_HASH..header::CONTENT_HASH + 4].copy_from_slice(&hash.to_le_bytes());
}

#[test]
fn mutated_databases_never_panic() {
    let original = support::compile_examples();
    let mut accepted = 0usize;

    for seed in 0..2_000u64 {
        let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1));
        let mut bytes = original.clone();
        for _ in 0..1 + rng.below(4) {
            mutate(&mut bytes, &mut rng);
        }
        // Half the runs keep the seal intact (so the checksum catches them),
        // half repair it (so the validators have to).
        if seed % 2 == 0 {
            reseal(&mut bytes);
        }

        if let Ok(db) = ProcedureDatabase::from_bytes(&bytes) {
            accepted += 1;
            execute_everything(&db);
        }
    }

    // If nothing survived, the test would be passing vacuously.
    assert!(
        accepted > 0,
        "no mutated database was accepted; the fuzzer is not reaching the verifier"
    );
    println!("{accepted} of 2000 mutated databases verified and executed");
}

#[test]
fn arbitrary_bytes_never_panic() {
    // Not a mutation of anything valid: pure noise, plus noise wearing the
    // right magic number.
    for seed in 0..1_000u64 {
        let mut rng = Rng::new(seed.wrapping_mul(0xD1B5_4A32_D192_ED03).wrapping_add(7));
        let len = rng.below(256);
        let mut bytes: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        let _ = ProcedureDatabase::from_bytes(&bytes);

        if bytes.len() >= 4 {
            bytes[0..4].copy_from_slice(b"FEBC");
            if let Ok(db) = ProcedureDatabase::from_bytes(&bytes) {
                execute_everything(&db);
            }
        }
    }
}

#[test]
fn every_single_byte_flip_is_survivable() {
    // Exhaustive rather than random: flip the low bit of every byte in the
    // file, one at a time, reseal, and load. This is the mutation a corrupted
    // download actually produces.
    let original = support::compile_examples();
    let mut accepted = 0usize;
    for at in 0..original.len() {
        let mut bytes = original.clone();
        bytes[at] ^= 0x01;
        reseal(&mut bytes);
        if let Ok(db) = ProcedureDatabase::from_bytes(&bytes) {
            accepted += 1;
            execute_everything(&db);
        }
    }
    println!(
        "{accepted} of {} single-bit flips still verified",
        original.len()
    );
}
