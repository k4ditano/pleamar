// El cristal mira hacia dónde apunta el borde con `dpdx`/`dpdy` de la
// distancia. Todos los píxeles de un quad son del mismo elemento, así que las
// ramas por tipo no parten un cuadro de 2×2 en dos: la derivada vale.
diagnostic(off, derivative_uniformity);

// Un quad por elemento. Cada elemento trae su caja envolvente, y un píxel solo
// ejecuta las formas del elemento que lo cubre: la escena puede crecer sin que
// cada píxel pague por toda ella.
//
// Un elemento es un cuerpo —una o varias formas fundidas por un mínimo suave,
// con su pintura, borde, luz y sombra— o un trozo del atlas. Todo son
// distancias con signo y alfa premultiplicado.

struct U {
    cab: vec4<f32>,    // ancho y alto lógicos, tiempo, escala (píxeles de verdad por píxel lógico)
    hud: vec4<f32>,    // origen x, periodo en ms, lógica bloqueada, origen y: desde dónde mira esta superficie la escena
    tiempos: array<vec4<f32>, 30>,
    detras: vec4<f32>, // x: hay fondo despejado que leer (la lente) · yz: dónde se pulsó · w: cuánta luz deja
};

struct Forma {
    a: vec4<f32>,      // tipo, fusión, trazo (0 = rellena), -
    b: vec4<f32>,      // centro x, y · media anchura, media altura (segmento: vector hasta el otro extremo)
    c: vec4<f32>,      // radio, giro, escala x, escala y (arco: radio, giro, media apertura, -; camino: primer punto, giro, cuántos, cerrado)
    t0: vec4<f32>,     // lo heredado, ya invertido: de pantalla a local (matriz 2×2)…
    t1: vec4<f32>,     // …su traslación, y cuánto estira las distancias
};

struct Elemento {
    cab: vec4<f32>,       // tipo, primera forma (o nº de capa), nº de formas (o «teñido»), alfa
    caja: vec4<f32>,      // x0, y0, x1, y1
    color0: vec4<f32>,    // r, g, b, filo
    color1: vec4<f32>,    // -, -, -, degradado (0 no, 1 lineal, 2 radial)
    linea: vec4<f32>,     // degradado: de (x, y) a (x, y); radial: centro (x, y), radio, -
    luz: vec4<f32>,       // cantidad, desde y, alto, grosor del borde
    borde: vec4<f32>,     // r, g, b, -
    sombra: vec4<f32>,    // dx, dy, difusa, alfa
    recortes: vec4<f32>,  // hasta cuatro formas a las que se recorta (-1 = ninguna)
    destino: vec4<f32>,   // textura: x, y, ancho, alto · degradado: primera parada, cuántas, -, -
    uv: vec4<f32>,
    t0: vec4<f32>,        // de pantalla a local, como en las formas
    t1: vec4<f32>,
};

const ELIPSE: u32 = 0u;
const CAJA: u32 = 1u;
const ARCO: u32 = 2u;
const SEGMENTO: u32 = 3u;
const CAMINO: u32 = 4u;

const CUERPO: u32 = 0u;
const TEXTURA: u32 = 1u;
const CAPA: u32 = 2u;
const INSTRUMENTOS: u32 = 9u;

const LEJOS: f32 = 1e6;

// 0 · la escena, igual para todas las superficies.
@group(0) @binding(0) var<storage, read> formas: array<Forma>;
@group(0) @binding(1) var<storage, read> elementos: array<Elemento>;
@group(0) @binding(2) var atlas: texture_2d<f32>;
@group(0) @binding(3) var muestreo: sampler;
// Los puntos de los caminos, en pares x, y, relativos al centro de cada uno.
@group(0) @binding(4) var<storage, read> puntos: array<f32>;
// Las paradas de los degradados: r, g, b y dónde cae cada una, en orden.
@group(0) @binding(5) var<storage, read> paradas: array<vec4<f32>>;
// 2 · lo de cada superficie: su tamaño y su escala.
@group(2) @binding(0) var<uniform> u: U;
// Los grupos con opacidad se pintan aparte, aquí, y se funden de una vez.
@group(1) @binding(0) var capas: texture_2d_array<f32>;
// 3 · lo que hay detrás del cristal, ya despejado: nítido y esmerilado.
@group(3) @binding(0) var detras_nitido: texture_2d<f32>;
@group(3) @binding(1) var detras_borroso: texture_2d<f32>;
@group(3) @binding(2) var detras_muestreo: sampler;

// La lente: cuánto del cristal es fondo doblado (el resto, lo de detrás tal cual,
// que el compositor pone debajo y deja despejar la foto siguiente) y cuánto tinte.
const LENTE_ALFA: f32 = 0.88;
const LENTE_TINTE: f32 = 0.22;
// El grosor del cristal, en anchos de bisel: cuánto dobla.
const LENTE_GROSOR: f32 = 1.4;

// Cuánto se corre lo de detrás a una distancia `dentro` del borde, con un bisel
// de ancho `bisel`. El canto tiene el perfil de un «squircle», ⁴√(1 − (1 − x)⁴):
// sube casi vertical y se aplana hacia dentro, y en el centro no dobla nada.
// Un rayo que baja en vertical entra por esa pendiente y se tuerce según Snell
// (índice 1,5); lo que se corre es lo que le queda de grosor por la tangente de
// lo que se ha torcido.
fn corrimiento(dentro: f32, bisel: f32) -> f32 {
    let x = clamp(dentro / bisel, 0.0, 1.0);
    let v = 1.0 - x;
    let v4 = v * v * v * v;
    let alto = pow(max(1.0 - v4, 0.0), 0.25);
    let pendiente = v * v * v * pow(max(1.0 - v4, 1e-4), -0.75);
    let entra = atan(pendiente);
    let sale = asin(sin(entra) / 1.5);
    return alto * bisel * LENTE_GROSOR * tan(entra - sale);
}

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
    let q = p - u.hud.xw;
    s.pos = vec4<f32>(q.x / u.cab.x * 2.0 - 1.0, 1.0 - q.y / u.cab.y * 2.0, 0.0, 1.0);
    s.elemento = i;
    return s;
}

fn a_local(p: vec2<f32>, t0: vec4<f32>, t1: vec4<f32>) -> vec2<f32> {
    return vec2<f32>(dot(t0.xy, p), dot(t0.zw, p)) + t1.xy;
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

// Un camino: la distancia al tramo más cercano, con signo si está cerrado (el
// número de vueltas dice si el punto está dentro). Es el SDF de un polígono.
fn camino(p: vec2<f32>, primero: u32, n: u32, cerrado: bool) -> f32 {
    if (n < 2u) { return LEJOS; }
    var mejor = LEJOS;
    var dentro = 1.0;
    let tramos = select(n - 1u, n, cerrado);
    for (var i = 0u; i < tramos; i++) {
        let j = (i + 1u) % n;
        let a = vec2<f32>(puntos[(primero + i) * 2u], puntos[(primero + i) * 2u + 1u]);
        let b = vec2<f32>(puntos[(primero + j) * 2u], puntos[(primero + j) * 2u + 1u]);
        let e = b - a;
        let w = p - a;
        let h = clamp(dot(w, e) / max(dot(e, e), 1e-6), 0.0, 1.0);
        mejor = min(mejor, length(w - e * h));
        if (cerrado) {
            // Cruces de la horizontal que pasa por el punto: par, fuera; impar, dentro.
            let baja = p.y >= a.y;
            let sube = p.y < b.y;
            let lado = e.x * w.y > e.y * w.x;
            if ((baja && sube && lado) || (!baja && !sube && !lado)) { dentro = -dentro; }
        }
    }
    return mejor * dentro;
}

// El color de un degradado en `t`: entre las dos paradas que lo rodean.
fn entre_paradas(primera: u32, n: u32, t: f32) -> vec3<f32> {
    var antes = paradas[primera];
    if (t <= antes.w || n < 2u) { return antes.rgb; }
    for (var i = 1u; i < n; i++) {
        let ahora = paradas[primera + i];
        if (t <= ahora.w) {
            let d = max(ahora.w - antes.w, 1e-4);
            return mix(antes.rgb, ahora.rgb, (t - antes.w) / d);
        }
        antes = ahora;
    }
    return antes.rgb;
}

fn min_suave(a: f32, b: f32, k: f32) -> f32 {
    if (k < 0.5 || a > LEJOS * 0.5 || b > LEJOS * 0.5) { return min(a, b); }
    let h = max(k - abs(a - b), 0.0) / k;
    return min(a, b) - h * h * k * 0.25;
}

fn distancia(k: u32, punto: vec2<f32>) -> f32 {
    let f = formas[k];
    // Primero lo heredado del grupo, luego el giro propio sobre su centro.
    let p = girar(a_local(punto, f.t0, f.t1), f.b.xy, f.c.y) - f.b.xy;
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
        case CAMINO: {
            d = camino(p, u32(f.c.x), u32(f.c.z), f.c.w > 0.5);
        }
        default: {}
    }
    // El trazo convierte cualquier forma en su contorno: un círculo en un aro.
    if (f.a.z > 0.0 && d < LEJOS * 0.5) {
        let tipo = u32(f.a.x);
        // Lo que ya es una línea solo se engorda; lo que encierra algo se queda en su contorno.
        let linea = tipo == ARCO || tipo == SEGMENTO || (tipo == CAMINO && f.c.w <= 0.5);
        if (linea) { d = d - f.a.z * 0.5; } else { d = abs(d) - f.a.z * 0.5; }
    }
    if (d > LEJOS * 0.5) { return d; }
    return d * f.t1.z;
}

// El borde se suaviza en tres cuartos de píxel DE VERDAD: a escala 2, la mitad
// de ancho en píxeles lógicos, o todo saldría borroso.
fn cubre(d: f32) -> f32 {
    let m = 0.75 / u.cab.w;
    return 1.0 - smoothstep(-m, m, d);
}

fn sobre(abajo: vec4<f32>, color: vec3<f32>, a: f32) -> vec4<f32> {
    return vec4<f32>(color * a, a) + abajo * (1.0 - a);
}

fn instrumentos(p: vec2<f32>) -> vec4<f32> {
    var c = vec4<f32>(0.0);
    let hp = p - vec2<f32>(60.0, u.cab.y - 68.0);
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
    let piloto = length(p - vec2<f32>(38.0, u.cab.y - 38.0)) - 5.0;
    let tono_piloto = mix(vec3<f32>(0.62, 0.84, 0.74), vec3<f32>(0.95, 0.3, 0.26), u.hud.z);
    return sobre(c, tono_piloto, cubre(piloto));
}

@fragment
fn fs(e: Salida) -> @location(0) vec4<f32> {
    // La posición llega en píxeles de verdad; la escena piensa en lógicos.
    let p = e.pos.xy / u.cab.w + u.hud.xw;
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
    if (tipo == CAPA) {
        return textureLoad(capas, vec2<i32>(e.pos.xy), i32(el.cab.y), 0) * alfa;
    }
    let local = a_local(p, el.t0, el.t1);
    if (tipo == TEXTURA) {
        let q = local;
        let uv01 = (q - el.destino.xy) / max(el.destino.zw, vec2<f32>(1.0));
        if (uv01.x < 0.0 || uv01.x > 1.0 || uv01.y < 0.0 || uv01.y > 1.0) { discard; }
        let t = textureSampleLevel(atlas, muestreo, mix(el.uv.xy, el.uv.zw, uv01), 0.0);
        // Teñido: el trozo es una máscara —una letra, un icono simbólico— y el color lo pone el elemento.
        if (el.cab.z > 0.5) { return vec4<f32>(el.color0.rgb * t.a, t.a) * alfa; }
        return t * alfa;
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
    // Cristal: de 0 a 1, en el hueco de `uv`, que un cuerpo no usa.
    let vidrio = el.uv.x;
    if (con_sombra) {
        // Su color son los tres huecos de `color1`; sin decir nada, negra.
        // Bajo un cristal la sombra no se ve a través: solo alrededor.
        let tapada = 1.0 - cubre(d) * vidrio;
        c = sobre(c, el.color1.rgb, el.sombra.w * alfa * tapada * (1.0 - smoothstep(-8.0, el.sombra.z, d_sombra)));
    }
    var tono = el.color0.rgb;
    if (el.color1.w > 0.5) {
        var t = 0.0;
        if (el.color1.w > 1.5) {
            // Radial: desde el centro hacia fuera, hasta el radio.
            t = clamp(length(local - el.linea.xy) / max(el.linea.z, 0.0001), 0.0, 1.0);
        } else {
            let eje = el.linea.zw - el.linea.xy;
            t = clamp(dot(local - el.linea.xy, eje) / max(dot(eje, eje), 0.0001), 0.0, 1.0);
        }
        tono = entre_paradas(u32(el.destino.x), u32(el.destino.y), t);
    }
    let luz = clamp(1.0 - (local.y - el.luz.y) / max(el.luz.z, 1.0), 0.0, 1.0) * el.luz.x;
    tono += vec3<f32>(luz) + vec3<f32>(el.color0.w) * smoothstep(-2.2, -0.4, d);
    // Hacia dónde apunta el borde: hacia donde crece la distancia.
    let g = vec2<f32>(dpdx(d), dpdy(d));
    let normal = g / max(length(g), 1e-6);
    let dentro = max(-d, 0.0);
    if (vidrio > 0.0 && u.detras.x > 0.5) {
        // Con lo de detrás a mano, el cristal es ese fondo doblado por el
        // bisel, esmerilado, con un poco de su tinte. Rojo, verde y azul se
        // doblan un pelo distinto: la franja de color de un canto de cristal.
        let bisel = select(16.0, el.uv.y, el.uv.y > 0.5);
        // Un cristal que aparece no se funde: empieza a doblar la luz.
        let corre = corrimiento(dentro, bisel) * u.cab.w * vidrio;
        let tam = u.cab.xy * u.cab.w;
        let q = e.pos.xy - normal * corre;
        let tq = normal * corre * 0.06;
        let esmerilado = vec3<f32>(
            textureSampleLevel(detras_borroso, detras_muestreo, (q - tq) / tam, 0.0).r,
            textureSampleLevel(detras_borroso, detras_muestreo, q / tam, 0.0).g,
            textureSampleLevel(detras_borroso, detras_muestreo, (q + tq) / tam, 0.0).b,
        );
        let nitido = vec3<f32>(
            textureSampleLevel(detras_nitido, detras_muestreo, (q - tq) / tam, 0.0).r,
            textureSampleLevel(detras_nitido, detras_muestreo, q / tam, 0.0).g,
            textureSampleLevel(detras_nitido, detras_muestreo, (q + tq) / tam, 0.0).b,
        );
        // En el bisel lo doblado se ve bastante nítido; hacia dentro, esmerilado.
        let en_bisel = 1.0 - clamp(dentro / bisel, 0.0, 1.0);
        let fondo = mix(esmerilado, nitido, smoothstep(0.0, 0.6, en_bisel) * 0.85);
        // Un cristal aviva un poco lo que deja ver.
        let gris = dot(fondo, vec3<f32>(0.299, 0.587, 0.114));
        let vivo = clamp(mix(vec3<f32>(gris), fondo, 1.18), vec3<f32>(0.0), vec3<f32>(1.0));
        // Lo que tiene que verse es este cristal. Pero el compositor pone debajo
        // de nuestro alfa lo de detrás de verdad, nítido, y se colaría como un
        // fantasma del texto de detrás: se resta de antemano, que lo sabemos.
        // Se ve: lo nuestro + (1 − α)·detrás = cristal · cobertura.
        let visto = cubre(d) * alfa * vidrio;
        let debajo = textureSampleLevel(detras_nitido, detras_muestreo, e.pos.xy / tam, 0.0).rgb;
        let a_lente = visto * LENTE_ALFA;
        let rgb = clamp(visto * (mix(vivo, tono, LENTE_TINTE) - (1.0 - LENTE_ALFA) * debajo), vec3<f32>(0.0), vec3<f32>(a_lente));
        c = vec4<f32>(rgb, a_lente) + c * (1.0 - a_lente);
        // Sobre algo claro, más tinte: lo que va escrito encima se sigue leyendo.
        let claro = smoothstep(0.45, 0.9, gris);
        c = sobre(c, tono, claro * 0.35 * visto);
    } else {
        // Sin él, el relleno es un tinte: deja ver lo que hay detrás (que el
        // compositor desenfoca) y guarda su color.
        c = sobre(c, tono, cubre(d) * alfa * mix(1.0, 0.36, vidrio));
    }
    if (vidrio > 0.0) {
        // La luz viene de arriba a la izquierda. El canto que la mira brilla
        // fino y fuerte; el de enfrente, más flojo: es la luz que sale.
        let hacia_la_luz = normalize(vec2<f32>(-0.55, -0.83));
        let canto = 1.0 - smoothstep(0.4, 3.0, dentro);
        let mira = pow(max(dot(normal, hacia_la_luz), 0.0), 1.6);
        let sale = pow(max(-dot(normal, hacia_la_luz), 0.0), 1.6);
        // Y cuanto más cerca del borde, más claro: el cristal visto de canto.
        let fresnel = exp(-dentro / 10.0);
        let brillo = canto * (0.85 * mira + 0.4 * sale + 0.12) + 0.14 * fresnel;
        c = sobre(c, vec3<f32>(1.0), clamp(brillo, 0.0, 1.0) * vidrio * cubre(d) * alfa);
        // Al pulsarlo, el cristal se ilumina desde el dedo, y la luz se extiende.
        let lejos = distance(p, u.detras.yz);
        let luz_dedo = u.detras.w * exp(-lejos * lejos / (2.0 * 70.0 * 70.0));
        c = sobre(c, vec3<f32>(1.0), luz_dedo * 0.3 * vidrio * cubre(d) * alfa);
    }
    if (el.luz.w > 0.0) {
        c = sobre(c, el.borde.rgb, cubre(abs(d + el.luz.w * 0.5) - el.luz.w * 0.5) * alfa);
    }
    return c;
}
