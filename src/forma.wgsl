// Un intérprete, no una escena. Recorre por píxel la lista de instrucciones que
// le llega y no sabe qué está pintando: acumula distancias con signo, las funde
// con un mínimo suave y compone en alfa premultiplicado.

struct U {
    cab: vec4<f32>,    // ancho, alto, tiempo, nº de instrucciones
    hud: vec4<f32>,    // visible, periodo en ms, lógica bloqueada, -
    tiempos: array<vec4<f32>, 30>,
};

struct Instr {
    cab: vec4<f32>,    // operación, tipo de forma, fusión, (filo | margen)
    geo: vec4<f32>,    // forma: cx, cy, mitad x, mitad y · textura: x, y, ancho, alto
    geo2: vec4<f32>,   // radio, escala x, escala y, alfa
    color: vec4<f32>,  // r, g, b, cantidad de luz
    extra: vec4<f32>,  // grupo: sombra dx, dy, difusa, alfa · relleno: luz y0, alto · textura: uv
};

const GRUPO: u32 = 0u;
const FORMA: u32 = 1u;
const RELLENO: u32 = 2u;
const RECORTE: u32 = 3u;
const PLANO: u32 = 4u;
const TEXTURA: u32 = 5u;

const LEJOS: f32 = 1e6;

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read> lista: array<Instr>;
@group(0) @binding(2) var atlas: texture_2d<f32>;
@group(0) @binding(3) var muestreo: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i) / 2) * 4.0 - 1.0;
    let y = f32(i32(i) % 2) * 4.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
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

fn distancia(i: Instr, p: vec2<f32>) -> f32 {
    if (i.cab.y < 0.5) {
        let e = i.geo2.yz;
        return (length((p - i.geo.xy) / e) - i.geo2.x) * min(e.x, e.y);
    }
    if (i.geo.z < 0.5 || i.geo.w < 0.5) { return LEJOS; }
    return caja(p - i.geo.xy, i.geo.zw, i.geo2.x);
}

fn cubre(d: f32) -> f32 { return 1.0 - smoothstep(-0.75, 0.75, d); }

fn sobre(abajo: vec4<f32>, color: vec3<f32>, a: f32) -> vec4<f32> {
    return vec4<f32>(color * a, a) + abajo * (1.0 - a);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = pos.xy;
    var c = vec4<f32>(0.0);

    var d = LEJOS;          // el cuerpo que se va acumulando
    var d_sombra = LEJOS;   // el mismo cuerpo, visto desde donde cae su sombra
    var sombra = vec4<f32>(0.0);
    var recorte = 1.0;

    let n = u32(u.cab.w);
    for (var k = 0u; k < n; k++) {
        let i = lista[k];
        switch u32(i.cab.x) {
            case GRUPO: {
                d = LEJOS;
                d_sombra = LEJOS;
                sombra = i.extra;
            }
            case FORMA: {
                d = min_suave(d, distancia(i, p), i.cab.z);
                d_sombra = min_suave(d_sombra, distancia(i, p - sombra.xy), i.cab.z);
            }
            case RELLENO: {
                let a = i.geo2.w * recorte;
                if (sombra.w > 0.0) {
                    c = sobre(c, vec3<f32>(0.0), sombra.w * a * (1.0 - smoothstep(-8.0, sombra.z, d_sombra)));
                }
                let luz = clamp(1.0 - (p.y - i.extra.x) / max(i.extra.y, 1.0), 0.0, 1.0) * i.color.w;
                let tono = i.color.rgb + vec3<f32>(luz) + vec3<f32>(i.cab.w) * smoothstep(-2.2, -0.4, d);
                c = sobre(c, tono, cubre(d) * a);
            }
            case RECORTE: {
                if (i.cab.y > 1.5) { recorte = 1.0; } else { recorte = cubre(distancia(i, p) + i.cab.w); }
            }
            case PLANO: {
                c = sobre(c, i.color.rgb, cubre(distancia(i, p)) * i.geo2.w * recorte);
            }
            case TEXTURA: {
                let uv01 = (p - i.geo.xy) / max(i.geo.zw, vec2<f32>(1.0));
                if (uv01.x >= 0.0 && uv01.x <= 1.0 && uv01.y >= 0.0 && uv01.y <= 1.0) {
                    let uv = mix(i.extra.xy, i.extra.zw, uv01);
                    let t = textureSampleLevel(atlas, muestreo, uv, 0.0) * (i.geo2.w * recorte);
                    c = t + c * (1.0 - t.a);
                }
            }
            default: {}
        }
    }

    // Instrumentos: no son parte de ninguna escena. Una barra por frame y el
    // piloto del hilo de lógica.
    if (u.hud.x > 0.5) {
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
        c = sobre(c, tono_piloto, cubre(piloto));
    }

    return c;
}
