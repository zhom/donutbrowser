"use client";

import { createContext, type ReactNode, useContext } from "react";

const FieldVariantContext = createContext<"default" | "soft">("default");

/** Inputs and selects inside take the soft, borderless look by default. */
export function SoftFields({ children }: { children: ReactNode }) {
  return (
    <FieldVariantContext.Provider value="soft">
      {children}
    </FieldVariantContext.Provider>
  );
}

export function useFieldVariant() {
  return useContext(FieldVariantContext);
}
