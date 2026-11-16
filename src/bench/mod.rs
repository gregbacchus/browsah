//! Benchmarks kept as a regression check for the rendering path (idle CPU, 60 fps, zero copy,
//! multi-tab cost, frame-rate caps). Run with `--bench=single|tabs|fps`; see docs/research/.

use crate::cli::{Bench, Options};
use std::rc::Rc;

mod single;
mod tabs;

pub fn run(opts: Rc<Options>) {
    match opts.bench {
        Bench::Single => single::run(opts),
        Bench::Tabs | Bench::Fps => tabs::start(opts),
        Bench::None => unreachable!("bench::run called without --bench"),
    }
}
