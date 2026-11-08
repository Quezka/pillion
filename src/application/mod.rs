//! What Pillion does: the Engine takes events (from the moped, the phone, the rider) and answers with Commands the
//! platform carries out. It talks to the outside world only through those Commands and the ports in `ports`.

pub mod demo;
pub mod engine;
pub mod locale;
pub mod nav;
pub mod ports;
