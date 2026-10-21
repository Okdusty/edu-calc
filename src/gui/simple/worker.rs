//! One serialized worker; GTK objects and the interpreter never cross threads.
use crate::qalc::{self, ClassroomAnswer, Options};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc,
};

pub enum Job {
    Calculate { id: u64, text: String, options: Options, commit: bool },
    Graph { id: u64, rows: Vec<(usize, String)>, options: Options, range: (f64, f64) },
}
pub type Curve = Result<(Vec<f64>, Vec<f64>), String>;
pub enum Reply {
    Calculate { id: u64, text: String, commit: bool, answer: Result<ClassroomAnswer, String> },
    Graph { id: u64, rows: Vec<(usize, Curve)> },
}

pub fn start(calc: Arc<AtomicU64>, graph: Arc<AtomicU64>) -> (mpsc::Sender<Job>, mpsc::Receiver<Reply>) {
    let (tx, rx) = mpsc::channel();
    let (out, results) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(job) = rx.recv() {
            let reply = match job {
                Job::Calculate { id, text, options, commit } => {
                    if !commit && id != calc.load(Ordering::Relaxed) {
                        continue;
                    }
                    let answer = qalc::classroom_eval(&text, &options);
                    Reply::Calculate { id, text, commit, answer }
                }
                Job::Graph { id, rows, options, range } => {
                    let mut result = vec![];
                    for (row, text) in rows {
                        if id != graph.load(Ordering::Relaxed) {
                            break;
                        }
                        let samples = super::model::expression(&text)
                            .and_then(|expr| qalc::classroom_curve(&expr, &options, range.0, range.1).map_err(|e| super::model::explain(&expr, &e)));
                        result.push((row, samples));
                    }
                    if id != graph.load(Ordering::Relaxed) {
                        continue;
                    }
                    Reply::Graph { id, rows: result }
                }
            };
            if out.send(reply).is_err() {
                break;
            }
        }
    });
    (tx, results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn obsolete_graph_jobs_are_discarded() {
        let (tx, rx) = start(Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(2)));
        for id in [1, 2] {
            tx.send(Job::Graph { id, rows: vec![(0, "x²".into())], options: Options::default(), range: (-2.0, 2.0) }).unwrap();
        }
        let Reply::Graph { id, rows } = rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap() else { panic!("expected graph") };
        assert_eq!(id, 2);
        assert!(rows[0].1.is_ok());
    }
}
