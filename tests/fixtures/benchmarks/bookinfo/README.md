# Istio Bookinfo benchmark

A trimmed copy of the Istio Bookinfo sample (Apache-2.0, see `LICENSE`), pinned at
istio/istio commit `4d78712a90bc461dc60804208d6abe5525ac001d`.

Kept: the four services' source (Python, Ruby, Node, Java), Dockerfiles and the
Kubernetes manifests (under `services/manifests`). Dropped: tests, static assets,
build files, Istio routing config.

`truth.yaml` is read from source and manifests (6 edges):

- productpage `productpage.py`: `details`, `reviews` and `ratings` hosts, each
  `"name" if os.environ.get("X_HOSTNAME") is None else ...`
- reviews `LibertyRestEndpoint.java`: `System.getenv("RATINGS_HOSTNAME") == null ? "ratings" : ...`
- `bookinfo-ratings-v2.yaml`: `MONGO_DB_URL: mongodb://mongodb:27017/test`
- `bookinfo-ratings-v2-mysql.yaml`: `MYSQL_DB_HOST: mysqldb`

The first run scored recall 0.00: no edge was found. Two gaps were real, and
neither is entity matching:

1. A host with a literal default when its environment setting is unset (Python,
   Java, plus the usual `environ.get(.., default)` and `process.env.X || default`).
   Only settings named like a host (`*_HOST`, `*_HOSTNAME`) count.
2. Manifest env values that are a URL (`mongodb://mongodb:27017/test`) or a bare
   host in a setting named like one (`MYSQL_DB_HOST: mysqldb`).

Both were added after seeing the misses, so the score is tuned here, as Sock Shop's
is. Trap tests in `src/collectors/source.rs` and `src/collectors/gather.rs` guard
precision (log levels, regions, `localhost`, empty defaults, ports, paths). The
ratchet in `tests/benchmark.rs` is set just below the current score.
