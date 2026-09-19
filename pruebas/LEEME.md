# Pruebas del lenguaje

Un `.plm` pequeño por cosa que el lenguaje promete. La primera línea dice qué se espera:

```
// espera: bien
// espera: fallo «las fichas de «rows» no tienen ningún campo»
```

`./probar.sh` los pasa todos por `pleamar --comprobar` y dice cuáles no cumplen. Un fallo esperado se comprueba por un trozo de su mensaje: así también se vigila que los errores sigan diciendo algo útil.

Cuando se arregla un fallo del lenguaje, su caso mínimo acaba aquí.
