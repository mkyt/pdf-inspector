import "@mantine/core/styles.css";
import { MantineProvider } from "@mantine/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { BackendContext } from "./api";
import { remoteFromLocation } from "./backend/remote";

// Documents do not change while open (no editing yet), so cached answers stay valid.
const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: Infinity, gcTime: 5 * 60_000, retry: false, refetchOnWindowFocus: false } },
});

const backend = remoteFromLocation();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <MantineProvider defaultColorScheme="auto">
      <QueryClientProvider client={queryClient}>
        <BackendContext.Provider value={backend}>
          <App />
        </BackendContext.Provider>
      </QueryClientProvider>
    </MantineProvider>
  </StrictMode>,
);
