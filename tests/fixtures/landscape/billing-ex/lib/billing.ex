defmodule Billing do
  # Calls the public shop edge by its hostname, not by a service name.
  def storefront, do: Req.get!("https://shop.acme-store.io/orders")
end
