# Propuesta — Plantilla en Español

> Cada propuesta debe estar **personalizada** al proyecto. Las plantillas genéricas son ignoradas.
> Longitud objetivo: **200-350 palabras**.
> Mercado: principalmente Workana, Malt, Soyfreelancer, clientes de LATAM y España.

---

## Estructura (5 párrafos)

### P1 — Demuestra que leíste el brief
> Cita un detalle *específico* del proyecto.

### P2 — Credibilidad (1-2 frases)
> Un proyecto previo relevante con números si es posible.

### P3 — Plan de ataque (3-4 puntos)
```
- Paso 1: ...
- Paso 2: ...
- Entregable: ...
```

### P4 — Diferenciador (IA)
> Tu ángulo: Claude como multiplicador de velocidad.

### P5 — Pregunta concreta
> Una pregunta que obligue al cliente a responder.

---

## Ejemplo completo

```
Hola [Nombre],

Mencionaste que tu integración actual con la API de Stripe pierde eventos cuando hay carga alta — eso casi siempre es un problema de cola e idempotencia, no de Stripe. Es algo que se resuelve en un sprint enfocado de 2-3 días.

Mi background: construí un sistema de sorteos en tiempo real que manejaba 10K+ req/s en producción para una empresa de gaming, y actualmente lidero la infraestructura CI/CD de un equipo SI en Taiwán que trabaja en iOS, IoT y sistemas 5G.

Mi enfoque para tu proyecto:
- Auditar el handler actual + analizar una semana de tráfico para confirmar el patrón de fallo
- Agregar una cola durable (SQS / Redis Streams según tu stack) con claves de idempotencia
- Sumar observabilidad (Sentry + dashboard custom) para detectar problemas antes que tus clientes
- Entregable: PR con tests + guía de despliegue + 2 semanas de soporte para bug fixes

Lo que me diferencia: trabajo con Claude como co-ingeniero, lo que significa que entrego en días lo que un contractor típico cotiza en semanas — pero mantienes el criterio arquitectónico senior, no solo código generado por IA.

Antes de cotizar: ¿el handler de webhooks está dentro de tu monolito principal o ya es un servicio independiente? Eso cambia el plan de despliegue.

Saludos,
— [Tu nombre]
```

---

## Notas de localización

- **LATAM** (México, Argentina, Colombia, Chile): tono más cercano, "vos/tú" según país. Workana usa "tú".
- **España**: más formal al inicio, "usted" no es necesario en tech, "tú" está bien. Usan "vale" en vez de "ok".
- **Precios**: USD para LATAM (estabilidad cambiaria), EUR para España.
- **Evita**: traducciones literales de inglés ("agendar una llamada" mejor que "schedular un meeting").

---

## Anti-patterns (NO hacer)

- ❌ "Estimado señor/señora, estoy muy interesado..."
- ❌ Listar todas las tecnologías que conoces
- ❌ Cotizar sin preguntar nada primero
- ❌ Usar Google Translate sin revisar — los clientes lo notan
- ❌ Decir "puedo hacerlo en 1 día por $30" (race to bottom)
