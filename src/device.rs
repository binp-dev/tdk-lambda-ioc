use ferrite::{AnyVariable, Context, Downcast, ReadVariable, WriteVariable};

pub struct Device {
    volt_real: WriteVariable<i32>,
    curr_real: WriteVariable<i32>,
    over_volt_set_point: ReadVariable<i32>,
    under_volt_set_point: ReadVariable<i32>,
    volt_set: ReadVariable<i32>,
    curr_set: ReadVariable<i32>,
}

fn take_var<V>(ctx: &mut Context, name: &str) -> V
where
    AnyVariable: Downcast<V>,
{
    log::debug!("take: {}", name);
    let var = ctx
        .registry
        .remove(name)
        .expect(&format!("No such name: {}", name));
    let dir = var.direction();
    let dty = var.data_type();
    var.downcast()
        .expect(&format!("Bad type, {:?}: {:?} expected", dir, dty))
}

impl Device {
    pub fn new(addr: u8, ctx: &mut Context) -> Self {
        Self {
            volt_real: take_var(ctx, &format!("PS{}:volt_real", addr)),
            curr_real: take_var(ctx, &format!("PS{}:curr_real", addr)),
            over_volt_set_point: take_var(ctx, &format!("PS{}:over_volt_set_point", addr)),
            under_volt_set_point: take_var(ctx, &format!("PS{}:under_volt_set_point", addr)),
            volt_set: take_var(ctx, &format!("PS{}:volt_set", addr)),
            curr_set: take_var(ctx, &format!("PS{}:curr_set", addr)),
        }
    }
}
