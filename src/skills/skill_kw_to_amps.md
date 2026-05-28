# skill_kw_to_amps

Converts electrical power in Kilowatts (kW) to electric current in Amperes (A) based on voltage and phase type.

## Parameters
- `kw` (number, required): The power in Kilowatts (e.g., 5, 10.5).
- `voltage` (number, required): The voltage in Volts (e.g., 110, 220, 380).
- `phase` (string, optional): The phase type, either "single" or "three" (defaults to "single").
- `powerFactor` (number, optional): The power factor between 0.1 and 1.0 (defaults to 1.0).

## Code
```javascript
const p = parseFloat(kw);
const v = parseFloat(voltage);
const pf = powerFactor ? parseFloat(powerFactor) : 1.0;
const phaseType = phase ? phase.toLowerCase() : 'single';

if (isNaN(p) || isNaN(v) || isNaN(pf)) {
  throw new Error('Invalid numeric parameters provided.');
}

if (pf <= 0 || pf > 1) {
  throw new Error('Power factor must be between 0.1 and 1.0');
}

let amps = 0;
if (phaseType === 'three') {
  // Formula for 3-Phase: I = (kW * 1000) / (V * sqrt(3) * PF)
  amps = (p * 1000) / (v * Math.sqrt(3) * pf);
} else {
  // Formula for 1-Phase: I = (kW * 1000) / (V * PF)
  amps = (p * 1000) / (v * pf);
}

const formattedAmps = amps.toFixed(2);
return `Conversion results:\n- Power: ${p} kW\n- Voltage: ${v} V\n- Phase: ${phaseType}\n- Power Factor: ${pf}\n\nCalculated Current: ${formattedAmps} A`;
```
