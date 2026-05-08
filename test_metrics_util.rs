use metrics_util::debugging::{DebuggingRecorder, Snapshot};
use metrics::{counter, histogram, gauge};

fn main() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    recorder.install().unwrap();

    counter!("my_counter", "label" => "value").increment(1);
    
    let snapshot = snapshotter.snapshot();
    for (key, unit, description, value) in snapshot.into_vec() {
        println!("{:?} {:?}", key.name(), value);
    }
}