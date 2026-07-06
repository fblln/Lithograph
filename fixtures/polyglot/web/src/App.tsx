import React from "react";

type RouteSummary = {
  name: string;
  distanceKm: number;
};

const route: RouteSummary = {
  name: "Morning climb",
  distanceKm: 12.4,
};

export function App() {
  return (
    <main>
      <h1>{route.name}</h1>
      <p>{route.distanceKm} km</p>
    </main>
  );
}

