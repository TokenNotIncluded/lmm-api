// Independent compilation until the relay owner wires `pub mod routing` in lib.rs.
// Keeping the bridge here avoids changing shared entry points in task 04.
// Exact ratio contracts come from the real Core trust-pricing domain.
pub use lmm_core::billing;
#[path = "../src/routing/mod.rs"]
pub mod routing;

#[path = "../src/upstream_cost.rs"]
pub mod upstream_cost;
