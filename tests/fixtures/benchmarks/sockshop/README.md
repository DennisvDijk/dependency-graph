# Sock Shop benchmark

A trimmed copy of the Weaveworks Sock Shop microservices demo
(Apache-2.0, see `LICENSE`). Upstream is one repository per service plus a
deploy repository, all pinned here:

| Part | Upstream commit |
|---|---|
| microservices-demo (manifests) | `9dff06fae4981921caec6a62393a6ebfce4b3e3f` |
| front-end | `52dee656f4423be041619bbb090205f912e00113` |
| carts | `f4e80051668c6a83359c41b03a27baefa590adc8` |
| orders | `546a10c92d1ff9998c21f3849f5b95a0f14430a0` |
| catalogue | `925e08ee17c28b94b87160e1ae5e03da7f61cf84` |
| payment | `384e33447dda5334187517dda09566c9a17fd6af` |
| shipping | `9c0fbfa31895dd029d44d3434de93f64cd71f87b` |
| user | `e1a79e77860734cacc3f64cabff1487e22a442a4` |
| queue-master | `7dc33721c765e5af415c9d244d84637ff2d510cd` |

Kept: application source, `application.properties`, Dockerfiles and the
Kubernetes manifests (under `services/manifests`). Dropped: tests, API specs,
JSON, build files, static assets, docker-compose files.

Unlike Online Boutique, the hosts are written in code and properties, not in
manifest env vars. So `truth.yaml` is read from source:

- front-end `api/endpoints.js` (`util.format("http://catalogue%s", domain)`)
- orders `OrdersConfigurationProperties.java` (`new Hostname("payment")`)
- `application.properties` (`mongodb://${db:carts-db}`, `spring.rabbitmq.host`)
- catalogue `main.go` (a SQL DSN with `tcp(catalogue-db:3306)`)
- the `shipping-task` queue name in shipping and queue-master

Two common Java idioms were read after the first run flagged them as misses:
`new Hostname("payment")` style host wrappers and Spring's
`rabbitTemplate.convertAndSend("shipping-task", ...)`. These patterns were added
after seeing this system, so the score is tuned here, as Boutique's is. Trap tests
in `src/collectors/source.rs` guard the precision side. The ratchet in
`tests/benchmark.rs` is set just below the current score.
