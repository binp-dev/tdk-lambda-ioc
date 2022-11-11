use ferrite::*;

struct Vars {
    pub ser_numb: ArrayVariable<u8, false, true, true>,
    pub volt_real: Variable<f64, false, true, true>,
    pub curr_real: Variable<f64, false, true, true>,
    pub over_volt_set_point: Variable<f64, true, true, false>,
    pub under_volt_set_point: Variable<f64, true, true, false>,
    pub volt_set: Variable<f64, true, true, false>,
    pub curr_set: Variable<f64, true, true, false>,
}

pub struct Device {
    addr: u8,
    vars: Vars,
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
    let info = var.info();
    var.downcast()
        .expect(&format!("Bad type, {:?} expected", info))
}

impl Vars {
    pub fn new(prefix: &str, ctx: &mut Context) -> Self {
        Self {
            ser_numb: take_var(ctx, &format!("{}ser_numb", prefix)),
            volt_real: take_var(ctx, &format!("{}volt_real", prefix)),
            curr_real: take_var(ctx, &format!("{}curr_real", prefix)),
            over_volt_set_point: take_var(ctx, &format!("{}over_volt_set_point", prefix)),
            under_volt_set_point: take_var(ctx, &format!("{}under_volt_set_point", prefix)),
            volt_set: take_var(ctx, &format!("{}volt_set", prefix)),
            curr_set: take_var(ctx, &format!("{}curr_set", prefix)),
        }
    }
}

impl Device {
    pub fn new(addr: u8, ctx: &mut Context) -> Self {
        let prefix = format!("PS{}:", addr);
        Self {
            addr,
            vars: Vars::new(&prefix, ctx),
        }
    }
}
