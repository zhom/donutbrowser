"use client";

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { Loader2 } from "lucide-react";
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
  const [regions, setRegions] = useState<LocationItem[]>([]);
  const [cities, setCities] = useState<LocationItem[]>([]);
  const [isps, setIsps] = useState<LocationItem[]>([]);

  const [selectedCountry, setSelectedCountry] = useState("");
  const [selectedRegion, setSelectedRegion] = useState("");
  const [selectedCity, setSelectedCity] = useState("");
  const [selectedIsp, setSelectedIsp] = useState("");
  const [proxyName, setProxyName] = useState("");

  const [isLoadingCountries, setIsLoadingCountries] = useState(false);
  const [isLoadingRegions, setIsLoadingRegions] = useState(false);
  const [isLoadingCities, setIsLoadingCities] = useState(false);
  const [isLoadingIsps, setIsLoadingIsps] = useState(false);
  const [isCreating, setIsCreating] = useState(false);

  const handleClose = useCallback(() => {
    setSelectedCountry("");
    setSelectedRegion("");
    setSelectedCity("");
    setSelectedIsp("");
    setProxyName("");
    setRegions([]);
    setCities([]);
    setIsps([]);
    onClose();
  }, [onClose]);

  // Fetch countries on mount
  useEffect(() => {
    if (!isOpen) return;
    setIsLoadingCountries(true);
    void invoke<LocationItem[]>("cloud_get_countries")
      .then((data) => {
        setCountries(data);
      })
      .catch((err) => {
        console.error("Failed to fetch countries:", err);
        toast.error("Failed to load countries");
      })
      .finally(() => {
        setIsLoadingCountries(false);
      });
  }, [isOpen]);

  // Fetch regions when country changes
  useEffect(() => {
    if (!selectedCountry) {
      setRegions([]);
      return;
    }
    setIsLoadingRegions(true);
    setSelectedRegion("");
    setSelectedCity("");
    setSelectedIsp("");
    setCities([]);
    setIsps([]);
    void invoke<LocationItem[]>("cloud_get_regions", {
      country: selectedCountry,
    })
      .then((data) => {
        setRegions(data);
      })
      .catch((err) => {
        console.error("Failed to fetch regions:", err);
      })
      .finally(() => {
        setIsLoadingRegions(false);
      });
  }, [selectedCountry]);

  // Fetch cities when country or region changes (cities can be loaded without region)
  useEffect(() => {
    if (!selectedCountry) {
      setCities([]);
      return;
    }
    setIsLoadingCities(true);
    setSelectedCity("");
    const args: { country: string; region?: string } = {
      country: selectedCountry,
    };
    if (selectedRegion) {
      args.region = selectedRegion;
    }
    void invoke<LocationItem[]>("cloud_get_cities", args)
      .then((data) => {
        setCities(data);
      })
      .catch((err) => {
        console.error("Failed to fetch cities:", err);
      })
      .finally(() => {
        setIsLoadingCities(false);
      });
  }, [selectedCountry, selectedRegion]);

  // Fetch ISPs when country/region/city changes
  useEffect(() => {
    if (!selectedCountry) {
      setIsps([]);
      return;
    }
    setIsLoadingIsps(true);
    setSelectedIsp("");
    const args: { country: string; region?: string; city?: string } = {
      country: selectedCountry,
    };
    if (selectedRegion) args.region = selectedRegion;
    if (selectedCity) args.city = selectedCity;
    void invoke<LocationItem[]>("cloud_get_isps", args)
      .then((data) => {
        setIsps(data);
      })
      .catch((err) => {
        console.error("Failed to fetch ISPs:", err);
      })
      .finally(() => {
        setIsLoadingIsps(false);
      });
  }, [selectedCountry, selectedRegion, selectedCity]);

  // Auto-generate name from selections
  useEffect(() => {
    const parts: string[] = [];
    const countryItem = countries.find((c) => c.code === selectedCountry);
    if (countryItem) parts.push(countryItem.name);
    const regionItem = regions.find((s) => s.code === selectedRegion);
    if (regionItem) parts.push(regionItem.name);
    const cityItem = cities.find((c) => c.code === selectedCity);
    if (cityItem) parts.push(cityItem.name);
    const ispItem = isps.find((i) => i.code === selectedIsp);
    if (ispItem) parts.push(ispItem.name);
    if (parts.length > 0) {
      setProxyName(parts.join(" - "));
    }
  }, [
    selectedCountry,
    selectedRegion,
    selectedCity,
    selectedIsp,
    countries,
    regions,
    cities,
    isps,
  ]);

  const handleCreate = useCallback(async () => {
    if (!selectedCountry || !proxyName.trim()) return;
    setIsCreating(true);
    try {
      await invoke("create_cloud_location_proxy", {
        name: proxyName.trim(),
        country: selectedCountry,
        region: selectedRegion || null,
        city: selectedCity || null,
        isp: selectedIsp || null,
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
  }, [
    selectedCountry,
    selectedRegion,
    selectedCity,
    selectedIsp,
    proxyName,
    handleClose,
  ]);

  const countryOptions = countries.map((c) => ({
    value: c.code,
    label: c.name,
  }));
  const regionOptions = regions.map((s) => ({ value: s.code, label: s.name }));
  const cityOptions = cities.map((c) => ({ value: c.code, label: c.name }));
  const ispOptions = isps.map((i) => ({ value: i.code, label: i.name }));

  const LoadingSpinner = () => (
    <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
  );

  return (
    <Dialog open={isOpen} onOpenChange={handleClose}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Create Location Proxy</DialogTitle>
          <DialogDescription>
            Create a geo-targeted proxy with a 24-hour sticky session
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {/* Country - always visible */}
          <div className="space-y-2">
            <Label className="flex items-center gap-2">
              Country (required)
              {isLoadingCountries && <LoadingSpinner />}
            </Label>
            <Combobox
              options={countryOptions}
              value={selectedCountry}
              onValueChange={setSelectedCountry}
              placeholder={
                isLoadingCountries ? "Loading countries..." : "Select country"
              }
              searchPlaceholder="Search countries..."
              disabled={isLoadingCountries}
            />
          </div>

          {/* Region - always visible, disabled until country is selected */}
          <div className="space-y-2">
            <Label className="flex items-center gap-2">
              Region (optional)
              {isLoadingRegions && <LoadingSpinner />}
            </Label>
            <Combobox
              options={regionOptions}
              value={selectedRegion}
              onValueChange={setSelectedRegion}
              placeholder={
                !selectedCountry
                  ? "Select a country first"
                  : isLoadingRegions
                    ? "Loading regions..."
                    : regionOptions.length === 0
                      ? "No regions available"
                      : "Select region"
              }
              searchPlaceholder="Search regions..."
              disabled={!selectedCountry || isLoadingRegions}
            />
          </div>

          {/* City - always visible, disabled until country is selected */}
          <div className="space-y-2">
            <Label className="flex items-center gap-2">
              City (optional)
              {isLoadingCities && <LoadingSpinner />}
            </Label>
            <Combobox
              options={cityOptions}
              value={selectedCity}
              onValueChange={setSelectedCity}
              placeholder={
                !selectedCountry
                  ? "Select a country first"
                  : isLoadingCities
                    ? "Loading cities..."
                    : cityOptions.length === 0
                      ? "No cities available"
                      : "Select city"
              }
              searchPlaceholder="Search cities..."
              disabled={!selectedCountry || isLoadingCities}
            />
          </div>

          {/* ISP - always visible, disabled until country is selected */}
          <div className="space-y-2">
            <Label className="flex items-center gap-2">
              ISP (optional)
              {isLoadingIsps && <LoadingSpinner />}
            </Label>
            <Combobox
              options={ispOptions}
              value={selectedIsp}
              onValueChange={setSelectedIsp}
              placeholder={
                !selectedCountry
                  ? "Select a country first"
                  : isLoadingIsps
                    ? "Loading ISPs..."
                    : ispOptions.length === 0
                      ? "No ISPs available"
                      : "Select ISP"
              }
              searchPlaceholder="Search ISPs..."
              disabled={!selectedCountry || isLoadingIsps}
            />
          </div>

          {/* Name */}
          <div className="space-y-2">
            <Label>Name</Label>
            <Input
              value={proxyName}
              onChange={(e) => {
                setProxyName(e.target.value);
              }}
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
