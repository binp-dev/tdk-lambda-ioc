use ferrite::*;

pub struct Device {
    ser_numb: WriteArrayVariable<u8>,
    volt_real: WriteVariable<f64>,
    curr_real: WriteVariable<f64>,
    over_volt_set_point: ReadVariable<f64>,
    under_volt_set_point: ReadVariable<f64>,
    volt_set: ReadVariable<f64>,
    curr_set: ReadVariable<f64>,
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
            ser_numb: take_var(ctx, &format!("PS{}:ser_numb", addr)),
            volt_real: take_var(ctx, &format!("PS{}:volt_real", addr)),
            curr_real: take_var(ctx, &format!("PS{}:curr_real", addr)),
            over_volt_set_point: take_var(ctx, &format!("PS{}:over_volt_set_point", addr)),
            under_volt_set_point: take_var(ctx, &format!("PS{}:under_volt_set_point", addr)),
            volt_set: take_var(ctx, &format!("PS{}:volt_set", addr)),
            curr_set: take_var(ctx, &format!("PS{}:curr_set", addr)),
        }
    }
}
