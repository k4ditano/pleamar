// Un quad por elemento. Cada elemento trae su caja envolvente, y un píxel solo
// ejecuta las formas del elemento que lo cubre: la escena puede crecer sin que
// cada píxel pague por toda ella.
//
// Un elemento es un cuerpo —una o varias formas fundidas por un mínimo suave,
// con su pintura, borde, luz y sombra— o un trozo del atlas. Todo son
// distancias con signo y alfa premultiplicado.

struct U {
    cab: vec4<f32>,    // ancho, alto, tiempo, -
    hud: vec4<f32>,    // -, periodo en ms, lógica bloqueada, -
    tiempos: array<vec4<f32>, 30>,
};

struct Forma {
    a: vec4<f32>,      // tipo, fusión, trazo (0 = rellena), -
    b: vec4<f32>,      // centro x, y · media anchura, media altura (segmento: vector hasta el otro extremo)
    c: vec4<f32>,      // radio, giro, escala x, escala y (arco: radio, giro, media apertura, -)
    t: vec4<f32>,      // transformación heredada: pivote x, y, giro, -
};

struct Elemento {
    cab: vec4<f32>,       // tipo, primera forma, nº de formas, alfa
    caja: vec4<f32>,      // x0, y0, x1, y1
    color0: vec4<f32>,    // r, g, b, filo
    color1: vec4<f32>,    // r, g, b, degradado (0 no, 1 lineal)
    linea: vec4<f32>,     // degradado: de (x, y) a (x, y)
    luz: vec4<f32>,       // cantidad, desde y, alto, grosor del borde
    borde: vec4<f32>,     // r, g, b, -
    sombra: vec4<f32>,    // dx, dy, difusa, alfa
    recortes: vec4<f32>,  // hasta cuatro formas a las que se recorta (-1 = ninguna)
    destino: vec4<f32>,   // textura: x, y, ancho, alto
    uv: vec4<f32>,
    t: vec4<f32>,         // transformación de la textura: pivote x, y, giro, -
};

const ELIPSE: u32 = 0u;
const CAJA: u32 = 1u;
const ARCO: u32 = 2u;
const SEGMENTO: u32 = 3u;

const CUERPO: u32 = 0u;
const TEXTURA: u32 = 1u;
const INSTRUMENTOS: u32 = 9u;

const LEJOS: f32 = 1e6;

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> formas: array<Forma>;
@group(0) @binding(2) var<storage, read> elementos: array<Elemento>;
@group(0) @binding(3) var atlas: texture_2d<f32>;
@group(0) @binding(4) var muestreo: sampler;

struct Salida {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) elemento: u32,
};

@vertex
fn vs(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> Salida {
    var esquinas = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = elementos[i].caja;
    let p = mix(c.xy, c.zw, esquinas[v]);
    var s: Salida;
    s.pos = vec4<f32>(p.x / u.cab.x * 2.0 - 1.0, 1.0 - p.y / u.cab.y * 2.0, 0.0, 1.0);
    s.elemento = i;
    return s;
}

fn girar(p: vec2<f32>, pivote: vec2<f32>, angulo: f32) -> vec2<f32> {
    if (angulo == 0.0) { return p; }
    let q = p - pivote;
    let c = cos(angulo);
    let s = sin(angulo);
    return vec2<f32>(c * q.x + s * q.y, -s * q.x + c * q.y) + pivote;
}

fn caja(p: vec2<f32>, mitad: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - mitad + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn min_suave(a: f32, b: f32, k: f32) -> f32 {
    if (k < 0.5 || a > LEJOS * 0.5 || b > LEJOS * 0.5) { return min(a, b); }
    let h = max(k - abs(a - b), 0.0) / k;
    return min(a, b) - h * h * k * 0.25;
}

fn distancia(k: u32, punto: vec2<f32>) -> f32 {
    let f = formas[k];
    // Primero lo heredado del grupo, luego el giro propio sobre su centro.
    let p = girar(girar(punto, f.t.xy, f.t.z), f.b.xy, f.c.y) - f.b.xy;
    var d = LEJOS;
    switch u32(f.a.x) {
        case ELIPSE: {
            let e = f.c.zw;
            d = (length(p / e) - f.c.x) * min(e.x, e.y);
        }
        case CAJA: {
            if (f.b.z >= 0.5 && f.b.w >= 0.5) { d = caja(p, f.b.zw, f.c.x); }
        }
        case ARCO: {
            // Un arco como «∩», abierto lo que diga la media apertura.
            let sc = vec2<f32>(sin(f.c.z), cos(f.c.z));
            let q = vec2<f32>(abs(p.x), -p.y);
            if (sc.y * q.x > sc.x * q.y) { d = length(q - sc * f.c.x); } else { d = abs(length(q) - f.c.x); }
        }
        case SEGMENTO: {
            let h = clamp(dot(p, f.b.zw) / max(dot(f.b.zw, f.b.zw), 0.0001), 0.0, 1.0);
            d = length(p - f.b.zw * h);
        }
        default: {}
    }
    // El trazo convierte cualquier forma en su contorno: un círculo en un aro.
    if (f.a.z > 0.0 && d < LEJOS * 0.5) {
        if (u32(f.a.x) == ARCO || u32(f.a.x) == SEGMENTO) { d = d - f.a.z * 0.5; } else { d = abs(d) - f.a.z * 0.5; }
    }
    return d;
}

fn cubre(d: f32) -> f32 { return 1.0 - smoothstep(-0.75, 0.75, d); }

fn sobre(abajo: vec4<f32>, color: vec3<f32>, a: f32) -> vec4<f32> {
    return vec4<f32>(color * a, a) + abajo * (1.0 - a);
}

fn instrumentos(p: vec2<f32>) -> vec4<f32> {
    var c = vec4<f32>(0.0);
    let hp = p - vec2<f32>(60.0, 232.0);
    let fondo = caja(hp - vec2<f32>(286.0, 30.0), vec2<f32>(334.0, 38.0), 12.0);
    c = sobre(c, vec3<f32>(0.05, 0.055, 0.055), 0.78 * cubre(fondo));
    if (hp.x >= 0.0 && hp.x < 600.0 && hp.y >= 0.0 && hp.y <= 60.0) {
        let j = u32(hp.x / 5.0);
        let v = u.tiempos[j / 4u][j % 4u];
        let ms = abs(v);
        let alto = min(ms * 2.0, 60.0);
        let en_barra = step(60.0 - alto, hp.y) * step(fract(hp.x / 5.0), 0.8);
        var tono = vec3<f32>(0.62, 0.84, 0.74);
        if (ms > u.hud.y * 1.6) { tono = vec3<f32>(0.95, 0.35, 0.3); }
        if (v < 0.0) { c = sobre(c, vec3<f32>(0.95, 0.35, 0.3), 0.16); }
        c = sobre(c, tono, en_barra * step(0.01, ms));
        let linea = 60.0 - u.hud.y * 2.0;
        c = sobre(c, vec3<f32>(1.0), 0.25 * step(abs(hp.y - linea), 0.5));
    }
    let piloto = length(p - vec2<f32>(38.0, 262.0)) - 5.0;
    let tono_piloto = mix(vec3<f32>(0.62, 0.84, 0.74), vec3<f32>(0.95, 0.3, 0.26), u.hud.z);
    return sobre(c, tono_piloto, cubre(piloto));
}

@fragment
fn fs(e: Salida) -> @location(0) vec4<f32> {
    let p = e.pos.xy;
    let el = elementos[e.elemento];
    let tipo = u32(el.cab.x);
    if (tipo == INSTRUMENTOS) { return instrumentos(p); }

    var recorte = 1.0;
    for (var k = 0; k < 4; k++) {
        let r = el.recortes[k];
        if (r >= 0.0) { recorte *= cubre(distancia(u32(r), p)); }
    }
    let alfa = el.cab.w * recorte;
    if (alfa <= 0.001) { discard; }

    var c = vec4<f32>(0.0);
    if (tipo == TEXTURA) {
        let q = girar(p, el.t.xy, el.t.z);
        let uv01 = (q - el.destino.xy) / max(el.destino.zw, vec2<f32>(1.0));
        if (uv01.x < 0.0 || uv01.x > 1.0 || uv01.y < 0.0 || uv01.y > 1.0) { discard; }
        return textureSampleLevel(atlas, muestreo, mix(el.uv.xy, el.uv.zw, uv01), 0.0) * alfa;
    }

    let primera = u32(el.cab.y);
    let n = u32(el.cab.z);
    var d = LEJOS;
    var d_sombra = LEJOS;
    let con_sombra = el.sombra.w > 0.0;
    for (var k = 0u; k < n; k++) {
        let fusion = formas[primera + k].a.y;
        d = min_suave(d, distancia(primera + k, p), fusion);
        if (con_sombra) { d_sombra = min_suave(d_sombra, distancia(primera + k, p - el.sombra.xy), fusion); }
    }
    if (con_sombra) {
        c = sobre(c, vec3<f32>(0.0), el.sombra.w * alfa * (1.0 - smoothstep(-8.0, el.sombra.z, d_sombra)));
    }
    var tono = el.color0.rgb;
    if (el.color1.w > 0.5) {
        let eje = el.linea.zw - el.linea.xy;
        let t = clamp(dot(p - el.linea.xy, eje) / max(dot(eje, eje), 0.0001), 0.0, 1.0);
        tono = mix(el.color0.rgb, el.color1.rgb, t);
    }
    let luz = clamp(1.0 - (p.y - el.luz.y) / max(el.luz.z, 1.0), 0.0, 1.0) * el.luz.x;
    tono += vec3<f32>(luz) + vec3<f32>(el.color0.w) * smoothstep(-2.2, -0.4, d);
    c = sobre(c, tono, cubre(d) * alfa);
    if (el.luz.w > 0.0) {
        c = sobre(c, el.borde.rgb, cubre(abs(d + el.luz.w * 0.5) - el.luz.w * 0.5) * alfa);
    }
    return c;
}
