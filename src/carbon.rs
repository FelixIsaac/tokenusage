use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::types::{TokenCounts, UsageEvent};

/// Grid carbon intensity regions (gCO2e per kWh).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GridRegion {
    /// US East / Virginia (AWS us-east-1) - ~310 gCO2e/kWh
    #[default]
    UsEast,
    /// US West / Oregon Hydro - ~120 gCO2e/kWh
    UsWest,
    /// US National Average - ~368 gCO2e/kWh
    UsAverage,
    /// EU West / France Nuclear - ~55 gCO2e/kWh
    EuWest,
    /// Nordics / Iceland & Norway Hydro+Geothermal - ~15 gCO2e/kWh
    Nordic,
    /// Google Cloud 24/7 CFE matched average - ~100 gCO2e/kWh
    GoogleCfe,
    /// Global Grid Average - ~475 gCO2e/kWh
    Global,
}

impl GridRegion {
    pub fn gco2_per_kwh(&self) -> f64 {
        match self {
            Self::UsEast => 310.0,
            Self::UsWest => 120.0,
            Self::UsAverage => 368.0,
            Self::EuWest => 55.0,
            Self::Nordic => 15.0,
            Self::GoogleCfe => 100.0,
            Self::Global => 475.0,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::UsEast => "US-East (Virginia / AWS us-east-1)",
            Self::UsWest => "US-West (Oregon Hydro)",
            Self::UsAverage => "US Grid Average",
            Self::EuWest => "EU-West (France Low-Carbon)",
            Self::Nordic => "Nordics (100% Hydro/Geothermal)",
            Self::GoogleCfe => "Google Cloud 24/7 CFE",
            Self::Global => "Global Grid Average",
        }
    }
}

impl FromStr for GridRegion {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('_', "-").as_str() {
            "us-east" | "useast" | "virginia" => Ok(Self::UsEast),
            "us-west" | "uswest" | "oregon" => Ok(Self::UsWest),
            "us-avg" | "us-average" | "us" => Ok(Self::UsAverage),
            "eu-west" | "euwest" | "france" => Ok(Self::EuWest),
            "nordic" | "nordics" | "iceland" | "norway" => Ok(Self::Nordic),
            "google" | "google-cfe" | "cfe" => Ok(Self::GoogleCfe),
            "global" | "world" => Ok(Self::Global),
            _ => Err(format!("Unknown grid region '{s}'")),
        }
    }
}

/// Joules per token coefficients for model architectures.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModelCarbonCoefficients {
    pub input_joules_per_token: f64,
    pub cache_read_joules_per_token: f64,
    pub output_joules_per_token: f64,
}

impl ModelCarbonCoefficients {
    pub fn for_model(model_name: &str) -> Self {
        let name = model_name.to_lowercase();
        if name.contains("haiku")
            || name.contains("flash")
            || name.contains("mini")
            || name.contains("8b")
        {
            // Tier 1: Small / Fast (0.03 kWh / 1M input, 0.18 kWh / 1M output)
            Self {
                input_joules_per_token: 0.108,
                cache_read_joules_per_token: 0.020,
                output_joules_per_token: 0.648,
            }
        } else if name.contains("opus")
            || name.contains("405b")
            || name.contains("o1")
            || name.contains("o3")
        {
            // Tier 3: Heavy / Flagship / Reasoning (0.60 kWh / 1M input, 3.80 kWh / 1M output)
            Self {
                input_joules_per_token: 2.160,
                cache_read_joules_per_token: 0.400,
                output_joules_per_token: 13.680,
            }
        } else {
            // Tier 2: Frontier Workhorse (Claude 3.5 Sonnet, GPT-4o, Gemini 1.5 Pro)
            // 0.15 kWh / 1M input, 0.85 kWh / 1M output
            Self {
                input_joules_per_token: 0.540,
                cache_read_joules_per_token: 0.100,
                output_joules_per_token: 3.060,
            }
        }
    }
}

/// Calculated environmental metrics.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct EnvironmentalMetrics {
    /// Total electrical energy in kilowatt-hours (kWh)
    pub energy_kwh: f64,
    /// Carbon footprint in grams of CO2 equivalent (gCO2e)
    pub carbon_gco2e: f64,
    /// Total water footprint in milliliters (mL)
    pub water_ml: f64,
}

impl EnvironmentalMetrics {
    pub fn add(&mut self, other: &EnvironmentalMetrics) {
        self.energy_kwh += other.energy_kwh;
        self.carbon_gco2e += other.carbon_gco2e;
        self.water_ml += other.water_ml;
    }

    pub fn calculate(
        input_tokens: u64,
        cache_read_tokens: u64,
        output_tokens: u64,
        model_name: &str,
        grid: GridRegion,
        pue: f64,
        wue_combined: f64,
    ) -> Self {
        let coeffs = ModelCarbonCoefficients::for_model(model_name);

        let joules_it = (input_tokens as f64 * coeffs.input_joules_per_token)
            + (cache_read_tokens as f64 * coeffs.cache_read_joules_per_token)
            + (output_tokens as f64 * coeffs.output_joules_per_token);

        let kwh_it = joules_it / 3_600_000.0;
        let total_kwh = kwh_it * pue;

        // Amortized embodied carbon factor (~0.003 gCO2e per 1k total tokens)
        let total_tokens = input_tokens + cache_read_tokens + output_tokens;
        let embodied_gco2 = (total_tokens as f64 / 1000.0) * 0.003;
        let carbon_gco2e = (total_kwh * grid.gco2_per_kwh()) + embodied_gco2;

        // Water in mL = kWh * L/kWh * 1000 mL/L
        let water_ml = total_kwh * wue_combined * 1000.0;

        Self {
            energy_kwh: total_kwh,
            carbon_gco2e,
            water_ml,
        }
    }

    pub fn calculate_counts(
        counts: &TokenCounts,
        model_name: &str,
        grid: GridRegion,
        pue: f64,
        wue_combined: f64,
    ) -> Self {
        Self::calculate(
            counts.input_tokens + counts.cache_creation_input_tokens,
            counts.cache_read_input_tokens,
            counts.output_tokens + counts.reasoning_output_tokens,
            model_name,
            grid,
            pue,
            wue_combined,
        )
    }

    pub fn calculate_events(
        events: &[UsageEvent],
        grid: GridRegion,
        pue: f64,
        wue_combined: f64,
    ) -> Self {
        let mut total = Self::default();
        for event in events {
            let counts = event.usage.to_counts();
            let m = Self::calculate_counts(&counts, &event.model, grid, pue, wue_combined);
            total.add(&m);
        }
        total
    }
}

/// Intuitive human equivalences for energy, carbon, and water with dynamic scaling & wow factor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentalEquivalences {
    /// Number of full smartphone charges (~12 Wh per charge)
    pub smartphone_charges: f64,
    /// Cups of tea/coffee boiled in an electric kettle (~23 Wh per cup, 250ml 20°C->100°C)
    pub cups_boiled: f64,
    /// Kilometers driven in an Electric Vehicle (~150 Wh/km)
    pub ev_km: f64,
    /// Kilometers driven in a petrol car (~240 gCO2/km)
    pub petrol_car_km: f64,
    /// Standard 500ml water bottles evaporated
    pub water_bottles: f64,
    /// Tree-months equivalent (1 mature tree absorbs ~2,000 gCO2e per month)
    pub tree_months: f64,
    /// Hours of HD video streaming (~0.08 kWh/hour)
    pub streaming_hours: f64,
    /// Days of average US home power consumption (~30 kWh/day)
    pub home_power_days: f64,
    /// Transatlantic economy flights NYC -> London (~650 kg CO2e per passenger)
    pub nyc_london_flights: f64,
    /// Full standard 150-liter bathtubs of water
    pub bathtubs: f64,
}

impl EnvironmentalEquivalences {
    pub fn from_metrics(metrics: &EnvironmentalMetrics) -> Self {
        let carbon_kg = metrics.carbon_gco2e / 1000.0;
        let water_l = metrics.water_ml / 1000.0;

        Self {
            smartphone_charges: (metrics.energy_kwh * 1000.0) / 12.0,
            cups_boiled: (metrics.energy_kwh * 1000.0) / 23.0,
            ev_km: metrics.energy_kwh / 0.15,
            petrol_car_km: metrics.carbon_gco2e / 240.0,
            water_bottles: metrics.water_ml / 500.0,
            tree_months: metrics.carbon_gco2e / 2000.0,
            streaming_hours: metrics.energy_kwh / 0.08,
            home_power_days: metrics.energy_kwh / 30.0,
            nyc_london_flights: carbon_kg / 650.0,
            bathtubs: water_l / 150.0,
        }
    }

    /// Dynamic multi-tier human equivalences with "wow factor" scaling.
    pub fn wow_factor_summary_lines(&self, metrics: &EnvironmentalMetrics) -> Vec<String> {
        let mut lines = Vec::new();
        let kwh = metrics.energy_kwh;
        let carbon_kg = metrics.carbon_gco2e / 1000.0;
        let water_l = metrics.water_ml / 1000.0;

        if kwh >= 500.0 {
            lines.push(format!(
                "   ⚡ [Home Power] ~ {} Days powering an average US home (30 kWh/day)",
                format_commas_f64(self.home_power_days, 1)
            ));
        } else {
            lines.push(format!(
                "   📱 [Charging]   ~ {} Smartphone charges (12Wh each)",
                format_commas_f64(self.smartphone_charges, 0)
            ));
            lines.push(format!(
                "   ☕ [Kettle]     ~ {} Cups of tea / coffee boiled (23Wh each)",
                format_commas_f64(self.cups_boiled, 0)
            ));
        }

        if carbon_kg >= 300.0 {
            lines.push(format!(
                "   ✈️  [Flight CO₂] ~ {} Transatlantic flights (NYC ✈️ London, 650kg CO₂e each)",
                format_commas_f64(self.nyc_london_flights, 2)
            ));
        }

        let cross_country_trips = self.ev_km / 4000.0; // 4000 km ~ NYC to LA
        if self.ev_km >= 4000.0 {
            lines.push(format!(
                "   🚀 [EV Driving] ~ {} km in an EV (or {} km gas) — {:.1}× coast-to-coast US trips!",
                format_commas_f64(self.ev_km, 1),
                format_commas_f64(self.petrol_car_km, 1),
                cross_country_trips
            ));
        } else {
            lines.push(format!(
                "   🚘 [EV Driving] ~ {} km in an Electric Vehicle (or {} km in gas car)",
                format_commas_f64(self.ev_km, 1),
                format_commas_f64(self.petrol_car_km, 1)
            ));
        }

        if water_l >= 300.0 {
            lines.push(format!(
                "   🛁 [Water Draw] ~ {} Full bathtubs (150L each) or {} bottles",
                format_commas_f64(self.bathtubs, 1),
                format_commas_f64(self.water_bottles, 0)
            ));
        } else {
            lines.push(format!(
                "   💧 [Water Draw] ~ {} Drinking water bottles (500ml) evaporated",
                format_commas_f64(self.water_bottles, 1)
            ));
        }

        if self.tree_months >= 12.0 {
            let tree_years = self.tree_months / 12.0;
            lines.push(format!(
                "   🌳 [CO₂ Offset] ~ {} Tree-months ({:.1} full tree-years) of forest absorption",
                format_commas_f64(self.tree_months, 1),
                tree_years
            ));
        } else {
            lines.push(format!(
                "   🌳 [CO₂ Offset] ~ {} Tree-months of mature tree CO₂ absorption",
                format_commas_f64(self.tree_months, 1)
            ));
        }

        if self.streaming_hours >= 1000.0 {
            let streaming_years = self.streaming_hours / (24.0 * 365.25);
            lines.push(format!(
                "   📺 [Streaming]  ~ {} Hours ({:.1} years continuous) of HD video streaming",
                format_commas_f64(self.streaming_hours, 1),
                streaming_years
            ));
        } else {
            lines.push(format!(
                "   📺 [Streaming]  ~ {} Hours of HD video streaming",
                format_commas_f64(self.streaming_hours, 1)
            ));
        }

        lines
    }
}

/// Eco-efficiency rating based on carbon per 1k tokens.
pub fn eco_rating(gco2e_per_1k: f64) -> (&'static str, &'static str) {
    if gco2e_per_1k < 0.10 {
        ("A+", "Ultra-efficient (Small models / High prompt caching)")
    } else if gco2e_per_1k < 0.25 {
        ("A", "High efficiency")
    } else if gco2e_per_1k < 0.50 {
        ("B", "Moderate efficiency (Frontier workhorse models)")
    } else if gco2e_per_1k < 1.00 {
        ("C", "Below average efficiency")
    } else {
        (
            "D",
            "Resource intensive (Heavy flagship / reasoning models)",
        )
    }
}

/// Utility for formatting integers with thousand comma separators.
pub fn format_commas_u64(val: u64) -> String {
    let s = val.to_string();
    let mut result = String::new();
    let len = s.len();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}

/// Utility for formatting floats with thousand comma separators and fixed precision.
pub fn format_commas_f64(val: f64, decimals: usize) -> String {
    let formatted = format!("{:.1$}", val, decimals);
    let parts: Vec<&str> = formatted.split('.').collect();
    let int_part = parts[0];
    let is_negative = int_part.starts_with('-');
    let digits = if is_negative {
        &int_part[1..]
    } else {
        int_part
    };

    let mut formatted_int = String::new();
    let len = digits.len();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            formatted_int.push(',');
        }
        formatted_int.push(c);
    }

    let prefix = if is_negative { "-" } else { "" };
    if parts.len() > 1 && decimals > 0 {
        format!("{}{}.{}", prefix, formatted_int, parts[1])
    } else {
        format!("{}{}", prefix, formatted_int)
    }
}

/// Format USD currency cleanly with commas and 2 decimal places ($16,435.53).
pub fn format_currency(val: f64) -> String {
    format!("${}", format_commas_f64(val, 2))
}

/// Human-readable carbon footprint string (g or kg CO2e).
pub fn format_carbon_human(gco2: f64) -> String {
    if gco2 >= 1000.0 {
        format!("{} kg CO₂e", format_commas_f64(gco2 / 1000.0, 2))
    } else {
        format!("{} g CO₂e", format_commas_f64(gco2, 1))
    }
}

/// Human-readable water footprint string (mL or Liters).
pub fn format_water_human(ml: f64) -> String {
    if ml >= 1000.0 {
        format!("{} L", format_commas_f64(ml / 1000.0, 2))
    } else {
        format!("{} mL", format_commas_f64(ml, 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_region_gco2() {
        assert_eq!(GridRegion::UsEast.gco2_per_kwh(), 310.0);
        assert_eq!(GridRegion::Nordic.gco2_per_kwh(), 15.0);
        assert_eq!(GridRegion::EuWest.gco2_per_kwh(), 55.0);
    }

    #[test]
    fn test_eco_rating_grades() {
        assert_eq!(eco_rating(0.05).0, "A+");
        assert_eq!(eco_rating(0.15).0, "A");
        assert_eq!(eco_rating(0.35).0, "B");
        assert_eq!(eco_rating(0.75).0, "C");
        assert_eq!(eco_rating(1.50).0, "D");
    }

    #[test]
    fn test_format_commas_u64() {
        assert_eq!(format_commas_u64(0), "0");
        assert_eq!(format_commas_u64(999), "999");
        assert_eq!(format_commas_u64(1000), "1,000");
        assert_eq!(format_commas_u64(24418101550), "24,418,101,550");
    }

    #[test]
    fn test_format_commas_f64() {
        assert_eq!(format_commas_f64(1234.5678, 2), "1,234.57");
        assert_eq!(format_commas_f64(685833.6, 1), "685,833.6");
        assert_eq!(format_commas_f64(0.003, 3), "0.003");
    }

    #[test]
    fn test_format_currency() {
        assert_eq!(format_currency(427.1563), "$427.16");
        assert_eq!(format_currency(16435.5272), "$16,435.53");
    }

    #[test]
    fn test_human_formatters() {
        assert_eq!(format_carbon_human(500.0), "500.0 g CO₂e");
        assert_eq!(format_carbon_human(12500.0), "12.50 kg CO₂e");
        assert_eq!(format_water_human(450.0), "450 mL");
        assert_eq!(format_water_human(8497067.0), "8,497.07 L");
    }

    #[test]
    fn test_metrics_calculation() {
        let metrics = EnvironmentalMetrics::calculate(
            1_000_000,
            500_000,
            200_000,
            "claude-sonnet-5",
            GridRegion::UsEast,
            1.15,
            4.30,
        );
        assert!(metrics.energy_kwh > 0.0);
        assert!(metrics.carbon_gco2e > 0.0);
        assert!(metrics.water_ml > 0.0);

        let equiv = EnvironmentalEquivalences::from_metrics(&metrics);
        assert!(equiv.smartphone_charges > 0.0);
        assert!(equiv.cups_boiled > 0.0);
        assert!(equiv.tree_months > 0.0);
        assert!(equiv.streaming_hours > 0.0);
    }
}
