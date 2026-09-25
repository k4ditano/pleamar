// Lo que hay detrás del cristal, a partir de una foto de la pantalla.
//
// La foto se tomó con nuestra superficie encima, así que es la mezcla que hace
// el compositor: foto = lo nuestro + (1 − nuestro alfa) · fondo. Lo nuestro lo
// sabemos, porque es el último lienzo que se presentó: el fondo se despeja.
// Donde lo nuestro es casi opaco —el texto, un icono— no se puede, y se queda
// el fondo que había; lo mismo si el resultado no es un color posible, que es
// lo que pasa cuando la foto es de un frame distinto del lienzo (algo se movía).

struct Salida {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) v: u32) -> Salida {
    // Un triángulo que cubre la pantalla entera.
    let p = vec2<f32>(f32((v << 1u) & 2u), f32(v & 2u));
    var s: Salida;
    s.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    return s;
}

// ── despejar ──────────────────────────────────────────────────────
@group(0) @binding(0) var foto: texture_2d<f32>;
@group(0) @binding(1) var lienzo: texture_2d<f32>;
@group(0) @binding(2) var antes: texture_2d<f32>;
@group(0) @binding(3) var lineal: sampler;
// Dónde cae la foto en el lienzo, en píxeles: x, y, ancho, alto.
@group(0) @binding(4) var<uniform> caja: vec4<f32>;

@fragment
fn despejar(e: Salida) -> @location(0) vec4<f32> {
    let px = vec2<i32>(e.pos.xy);
    // La foto la hace el compositor a su escala: se lee en proporción.
    let f = textureSampleLevel(foto, lineal, (e.pos.xy - caja.xy) / caja.zw, 0.0).rgb;
    let s = textureLoad(lienzo, px, 0);
    let a = textureLoad(antes, px, 0);
    let queda = 1.0 - s.a;
    if (queda < 0.03) {
        return a;
    }
    let fondo = (f - s.rgb) / queda;
    // Un fondo que no puede ser: la foto no es de este lienzo.
    if (any(fondo < vec3<f32>(-0.03)) || any(fondo > vec3<f32>(1.03))) {
        return a;
    }
    return vec4<f32>(clamp(fondo, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

// ── desenfocar ────────────────────────────────────────────────────
// Gaussiana separable: una pasada en horizontal y otra en vertical.
struct Borrar {
    paso: vec2<f32>,    // (1, 0) o (0, 1), en píxeles
    sigma: f32,         // en píxeles
    _r: f32,
};
@group(0) @binding(0) var origen: texture_2d<f32>;
@group(0) @binding(1) var<uniform> b: Borrar;

@fragment
fn borrar(e: Salida) -> @location(0) vec4<f32> {
    let px = vec2<i32>(e.pos.xy);
    let lim = vec2<i32>(textureDimensions(origen)) - 1;
    let radio = i32(ceil(b.sigma * 2.5));
    var suma = vec4<f32>(0.0);
    var peso = 0.0;
    for (var k = -radio; k <= radio; k++) {
        let w = exp(-f32(k * k) / (2.0 * b.sigma * b.sigma));
        let q = clamp(px + vec2<i32>(b.paso * f32(k)), vec2<i32>(0), lim);
        suma += textureLoad(origen, q, 0) * w;
        peso += w;
    }
    return suma / peso;
}
