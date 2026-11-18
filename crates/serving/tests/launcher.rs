//! The pair launcher ends both its children when it is told to end, by
//! SIGTERM or SIGINT, and when either child ends.
//!
//! This test binary is its own launcher: run with `TRANSLATOR_PAIR_LAUNCHER`
//! in its environment it launches a pair from its arguments, as a
//! translator's pair binary does; run without it, it runs each case against
//! a launcher of itself. Each child is a shell script that prints its process
//! identifier on the launcher's standard output and then sleeps, so the test
//! reads both identifiers as they are written.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};

use nix::sys::signal::{Signal as ProcessSignal, kill};
use nix::unistd::Pid;
use shared_translator::placement::{Place, PlaceName, Runtime, State};
use translator_serving::{PairReading, PairRunning, Pairing};

const LAUNCHER: &str = "TRANSLATOR_PAIR_LAUNCHER";

fn main() -> ExitCode {
    if std::env::var_os(LAUNCHER).is_some() {
        let launched =
            Pairing::from_arguments(std::env::args().skip(1)).and_then(|pairing| pairing.launch());
        let Err(error) = launched;
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }
    for (name, case) in [
        (
            "a_launcher_ended_by_sigterm_ends_its_pair",
            Case::Told(ProcessSignal::SIGTERM),
        ),
        (
            "a_launcher_ended_by_sigint_ends_its_pair",
            Case::Told(ProcessSignal::SIGINT),
        ),
        (
            "a_nexus_that_ends_ends_its_translator",
            Case::Ended(Member::Nexus),
        ),
        (
            "a_translator_that_ends_ends_its_nexus",
            Case::Ended(Member::Translator),
        ),
    ] {
        case.run(name);
        println!("test {name} ... ok");
    }
    ExitCode::SUCCESS
}

#[derive(Clone, Copy)]
enum Member {
    Nexus,
    Translator,
}

/// What ends one launched pair.
#[derive(Clone, Copy)]
enum Case {
    /// The launcher is sent this signal.
    Told(ProcessSignal),
    /// This child is killed.
    Ended(Member),
}

/// A launcher of this binary, and the children it reported.
struct Launched {
    launcher: Child,
    nexus: Pid,
    translator: Pid,
}

impl Case {
    fn run(self, name: &str) {
        let directory = fixture(name);
        let mut launched = Launched::start(&directory);
        match self {
            Case::Told(signal) => {
                kill(launcher_pid(&launched.launcher), signal).expect("the launcher is signalled");
            }
            Case::Ended(member) => {
                kill(launched.member(member), ProcessSignal::SIGKILL).expect("the child is killed");
            }
        }
        let status = launched.launcher.wait().expect("the launcher ends");
        assert!(!status.success(), "{name}: the launch ends in an error");
        let mut reported = String::new();
        launched
            .launcher
            .stderr
            .take()
            .expect("launcher stderr")
            .read_to_string(&mut reported)
            .expect("launcher stderr read");
        let expected = match self {
            Case::Told(signal) => format!("launcher told to end: {signal}"),
            Case::Ended(Member::Nexus) => "paired process ended: the Nexus".to_owned(),
            Case::Ended(Member::Translator) => "paired process ended: the translator".to_owned(),
        };
        assert!(reported.contains(&expected), "{name}: {reported}");
        for (member, pid) in [
            ("daemon", launched.nexus),
            ("translator", launched.translator),
        ] {
            assert!(
                !Path::new(&format!("/proc/{pid}")).exists(),
                "{name}: the {member} ends with its launcher"
            );
        }
        let _ = std::fs::remove_dir_all(&directory);
    }
}

impl Launched {
    /// Starts a launcher of this binary with a pair of sleeping scripts, and
    /// reads each child's process identifier as it prints it.
    fn start(directory: &Path) -> Self {
        let place = Place {
            place_name: PlaceName("counter".into()),
            runtime: Runtime(directory.join("runtime").to_string_lossy().into_owned()),
            state: State(directory.join("state").to_string_lossy().into_owned()),
        };
        let nexus = script(directory, "nexus");
        let translator = script(directory, "translator");
        let mut launcher = Command::new(std::env::current_exe().expect("this binary"))
            .env(LAUNCHER, "1")
            .args([
                shared_translator::encode(&place),
                nexus.to_string_lossy().into_owned(),
                translator.to_string_lossy().into_owned(),
                "1000".into(),
                "4096".into(),
                "5".into(),
                "64".into(),
                "8".into(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the launcher starts");
        let mut lines = BufReader::new(launcher.stdout.take().expect("launcher stdout")).lines();
        let (mut nexus, mut translator) = (None, None);
        while nexus.is_none() || translator.is_none() {
            let line = lines
                .next()
                .expect("each child reports itself")
                .expect("a report line");
            let (member, pid) = line.split_once(' ').expect("a member and its pid");
            let pid = Pid::from_raw(pid.parse().expect("a pid"));
            match member {
                "nexus" => nexus = Some(pid),
                "translator" => translator = Some(pid),
                other => panic!("an unknown member reported: {other}"),
            }
        }
        Self {
            launcher,
            nexus: nexus.expect("the Nexus reported"),
            translator: translator.expect("the translator reported"),
        }
    }

    fn member(&self, member: Member) -> Pid {
        match member {
            Member::Nexus => self.nexus,
            Member::Translator => self.translator,
        }
    }
}

fn launcher_pid(launcher: &Child) -> Pid {
    Pid::from_raw(i32::try_from(launcher.id()).expect("a pid"))
}

/// A child of the pair: a script that reports itself and sleeps.
fn script(directory: &Path, member: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join(member);
    std::fs::write(
        &path,
        format!("#!/bin/sh\necho {member} $$\nexec sleep 600\n"),
    )
    .expect("the script is written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("the script is executable");
    path
}

fn fixture(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("translator-launcher-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("fixture directory");
    directory
}
