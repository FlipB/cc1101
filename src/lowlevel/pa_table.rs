use core::ops::Index;

/// PA_TABLE according to table 37 and table 39 on page 60 in cc1101 manual.
/// NOTE: when writing a pa table the spi write expects 8 bytes.
/// Also good to know, all values except for index 0 is dropped on SLEEP.
const PA_TABLE: [PaColumn; 4] = [
    PaColumn([0x12, 0x0D, 0x1C, 0x34, 0x51, 0x51, 0x85, 0xCB, 0xC2, 0xC2]), // 315 MHz
    PaColumn([0x12, 0x0E, 0x1D, 0x34, 0x60, 0x60, 0x84, 0xC8, 0xC0, 0xC0]), // 433 MHz
    PaColumn([0x03, 0x17, 0x1D, 0x26, 0x37, 0x50, 0x86, 0xCD, 0xC5, 0xC0]), // 868 MHz
    PaColumn([0x03, 0x0E, 0x1E, 0x27, 0x38, 0x8E, 0x84, 0xCC, 0xC3, 0xC0]), // 915 MHz
];

#[derive(Debug, Clone, Copy)]
pub struct PaColumn([u8; 10]);

impl PaColumn {
    /// output_power_dBm_row_index returns the row index closest matching provided output power Bm
    pub const fn output_power_value(&self, output_dbm: i32) -> u8 {
        let index = output_power_row_index(output_dbm);
        self.0[index]
    }

    /// values returns the condensed column entries suitable for writing to device.
    pub const fn values(&self) -> [u8; 8] {
        // Skip index 5 and 9 (alternatively we could skip 4 and 8).
        [
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[6], self.0[7], self.0[8],
        ]
    }
}

impl Index<usize> for PaColumn {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        self.0.index(index)
    }
}

/// pa_table gets the PA table column closest matching the provided frequency.
pub const fn pa_table(hz: u64) -> PaColumn {
    let table = PA_TABLE[table_column_index(hz)];
    table
}

/// table_column_index returns the column index closest to the provided frequency -
/// even if radio is incapable of operating at the frequency.
const fn table_column_index(hz: u64) -> usize {
    // TODO check how we arrived at these ranges - might not be intuitive.
    match hz {
        u64::MIN..=363_000_000 => 0,    // 315 MHz
        363_000_001..=621_500_000 => 1, // 433 MHz
        621_500_001..=899_990_000 => 2, // 868 MHz
        899_990_001..=u64::MAX => 3,    // 915 MHz
    }
}

/// output_power_dBm_row_index returns the row index closest matching provided output power Bm
pub const fn output_power_row_index(output_dbm: i32) -> usize {
    match output_dbm {
        i32::MIN..=-30 => 0,
        -31..=-20 => 1,
        -21..=-15 => 2,
        -16..=-10 => 3,
        -11..=-6 => 4, // This and the following was merged.
        -5..=0 => 5,   //
        1..=5 => 6,
        6..=7 => 7,
        8..=10 => 8,        // This and the following was merged.
        11..=i32::MAX => 9, //
    }
}
