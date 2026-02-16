"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Combobox } from "@/components/ui/combobox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { LocationItem } from "@/types";
import { RippleButton } from "./ui/ripple";

interface LocationProxyDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

export function LocationProxyDialog({
  isOpen,
  onClose,
}: LocationProxyDialogProps) {
  const [countries, setCountries] = useState<LocationItem[]>([]);
  const [states, setStates] = useState<LocationItem[]>([]);
  const [cities, setCities] = useState<LocationItem[]>([]);

  const [selectedCountry, setSelectedCountry] = useState("");
  const [selectedState, setSelectedState] = useState("");
  const [selectedCity, setSelectedCity] = useState("");
  const [proxyName, setProxyName] = useState("");

  const [isLoadingCountries, setIsLoadingCountries] = useState(false);
  const [isLoadingStates, setIsLoadingStates] = useState(false);
  const [isLoadingCities, setIsLoadingCities] = useState(false);
  const [isCreating, setIsCreating] = useState(false);

  const handleClose = useCallback(() => {
    setSelectedCountry("");
    setSelectedState("");
    setSelectedCity("");
    setProxyName("");
    setStates([]);
    setCities([]);
    onClose();
  }, [onClose]);

  // Fetch countries on mount
  useEffect(() => {
    if (!isOpen) return;
    setIsLoadingCountries(true);
    invoke<LocationItem[]>("cloud_get_countries")
      .then((data) => setCountries(data))
      .catch((err) => {
        console.error("Failed to fetch countries:", err);
        toast.error("Failed to load countries");
      })
      .finally(() => setIsLoadingCountries(false));
  }, [isOpen]);

  // Fetch states when country changes
  useEffect(() => {
    if (!selectedCountry) {
      setStates([]);
      return;
    }
    setIsLoadingStates(true);
    setSelectedState("");
    setSelectedCity("");
    setCities([]);
    invoke<LocationItem[]>("cloud_get_states", { country: selectedCountry })
      .then((data) => setStates(data))
      .catch((err) => console.error("Failed to fetch states:", err))
      .finally(() => setIsLoadingStates(false));
  }, [selectedCountry]);

  // Fetch cities when state changes
  useEffect(() => {
    if (!selectedCountry || !selectedState) {
      setCities([]);
      return;
    }
    setIsLoadingCities(true);
    setSelectedCity("");
    invoke<LocationItem[]>("cloud_get_cities", {
      country: selectedCountry,
      state: selectedState,
    })
      .then((data) => setCities(data))
      .catch((err) => console.error("Failed to fetch cities:", err))
      .finally(() => setIsLoadingCities(false));
  }, [selectedCountry, selectedState]);

  // Auto-generate name from selections
  useEffect(() => {
    const parts: string[] = [];
    const countryItem = countries.find((c) => c.code === selectedCountry);
    if (countryItem) parts.push(countryItem.name);
    const stateItem = states.find((s) => s.code === selectedState);
    if (stateItem) parts.push(stateItem.name);
    const cityItem = cities.find((c) => c.code === selectedCity);
    if (cityItem) parts.push(cityItem.name);
    if (parts.length > 0) {
      setProxyName(parts.join(" - "));
    }
  }, [selectedCountry, selectedState, selectedCity, countries, states, cities]);

  const handleCreate = useCallback(async () => {
    if (!selectedCountry || !proxyName.trim()) return;
    setIsCreating(true);
    try {
      await invoke("create_cloud_location_proxy", {
        name: proxyName.trim(),
        country: selectedCountry,
        state: selectedState || null,
        city: selectedCity || null,
      });
      toast.success("Location proxy created");
      await emit("stored-proxies-changed");
      handleClose();
    } catch (error) {
      console.error("Failed to create location proxy:", error);
      toast.error(
        typeof error === "string" ? error : "Failed to create location proxy",
      );
    } finally {
      setIsCreating(false);
    }
  }, [selectedCountry, selectedState, selectedCity, proxyName, handleClose]);

  const countryOptions = countries.map((c) => ({
    value: c.code,
    label: c.name,
  }));
  const stateOptions = states.map((s) => ({ value: s.code, label: s.name }));
  const cityOptions = cities.map((c) => ({ value: c.code, label: c.name }));

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Create Location Proxy</DialogTitle>
          <DialogDescription>
            Create a geo-targeted proxy from your cloud credentials
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label>Country (required)</Label>
            <Combobox
              options={countryOptions}
              value={selectedCountry}
              onValueChange={setSelectedCountry}
              placeholder={isLoadingCountries ? "Loading..." : "Select country"}
              searchPlaceholder="Search countries..."
            />
          </div>

          {selectedCountry && stateOptions.length > 0 && (
            <div className="space-y-2">
              <Label>State (optional)</Label>
              <Combobox
                options={stateOptions}
                value={selectedState}
                onValueChange={setSelectedState}
                placeholder={isLoadingStates ? "Loading..." : "Select state"}
                searchPlaceholder="Search states..."
              />
            </div>
          )}

          {selectedState && cityOptions.length > 0 && (
            <div className="space-y-2">
              <Label>City (optional)</Label>
              <Combobox
                options={cityOptions}
                value={selectedCity}
                onValueChange={setSelectedCity}
                placeholder={isLoadingCities ? "Loading..." : "Select city"}
                searchPlaceholder="Search cities..."
              />
            </div>
          )}

          <div className="space-y-2">
            <Label>Name</Label>
            <Input
              value={proxyName}
              onChange={(e) => setProxyName(e.target.value)}
              placeholder="Proxy name"
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={handleClose}>
            Cancel
          </Button>
          <RippleButton
            onClick={handleCreate}
            disabled={!selectedCountry || !proxyName.trim() || isCreating}
          >
            {isCreating ? "Creating..." : "Create"}
          </RippleButton>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
