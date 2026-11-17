defmodule Billing.Pipeline do
  use Broadway

  def start_link(_opts) do
    Broadway.start_link(__MODULE__,
      producer: [module: {BroadwayKafka.Producer, topics: ["order-events"]}]
    )
  end
end
