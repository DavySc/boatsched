// All user-facing Dutch strings that more than one component needs. Component-
// local labels stay inline.

export const DAY_ORDER = ["Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];

export const DAY_NL: Record<string, string> = {
  Fri: "Vrijdag",
  Sat: "Zaterdag",
  Sun: "Zondag",
  Mon: "Maandag",
  Tue: "Dinsdag",
  Wed: "Woensdag",
};

export const DAY_NL_SHORT: Record<string, string> = {
  Fri: "Vri",
  Sat: "Zat",
  Sun: "Zon",
  Mon: "Maa",
  Tue: "Din",
  Wed: "Woe",
};

export const dayName = (d: string) => DAY_NL[d] ?? d;

export const LEVEL_LABEL: Record<string, string> = {
  k: "Ketelbinkie",
  b: "Bemanning",
  A: "A",
  B: "B",
};

export const levelLabel = (l: string) => LEVEL_LABEL[l] ?? l;

export const sortDays = (days: string[]) =>
  [...days].sort((a, b) => {
    const ia = DAY_ORDER.indexOf(a);
    const ib = DAY_ORDER.indexOf(b);
    return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib);
  });
