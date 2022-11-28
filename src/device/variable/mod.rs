mod device;
mod iface;

pub use device::*;
pub use iface::*;

use ferrite::VarSync;

pub struct Binding<T, V: VarSync, A: Adapter<T, V>, P: Parser<T>> {
    front: IfaceVariable<T, V, A>,
    back: DeviceVariable<T, P>,
}
