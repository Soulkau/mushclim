use core::{
    iter::Iterator,
    option::{Option, Option::*},
    result::{
        Result,
        Result::{Err, Ok},
    },
};
use embassy_time::{Duration, TimeoutError, Timer, with_timeout};
use esp_hal::i2c::master::I2c;

pub const LCD_ADDRESS: u8 = 0x3E; // LCD display
pub const RGB_ADDRESS: u8 = 0x2D; // RGB backlight (V2.0)

// LCD Commands
const LCD_CLEARDISPLAY: u8 = 0x01;
const LCD_RETURNHOME: u8 = 0x02;
const LCD_ENTRYMODESET: u8 = 0x04;
const LCD_DISPLAYCONTROL: u8 = 0x08;
const LCD_FUNCTIONSET: u8 = 0x20;
const LCD_SETDDRAMADDR: u8 = 0x80;
// Flags for display on/off control
const LCD_DISPLAYON: u8 = 0x04;
const LCD_CURSOROFF: u8 = 0x00;
const LCD_BLINKOFF: u8 = 0x00;

// Flags for function set
const LCD_2LINE: u8 = 0x08;
const LCD_5X8DOTS: u8 = 0x00;

// Flags for entry mode
const LCD_ENTRYLEFT: u8 = 0x02;
const LCD_ENTRYSHIFTDECREMENT: u8 = 0x00;

// RGB registers
const REG_RED: u8 = 0x04;
const REG_GREEN: u8 = 0x03;
const REG_BLUE: u8 = 0x02;
const REG_MODE1: u8 = 0x00;
const REG_MODE2: u8 = 0x01;
const REG_OUTPUT: u8 = 0x08;

// How long we'll wait on any single I2C op before giving up.
// Bus stuck / device unplugged -> this fires instead of hanging forever.
const I2C_TIMEOUT: Duration = Duration::from_millis(100);

/// Row offsets for up to 4-line HD44780-style displays.
/// (16x4 / 20x4 displays use this exact split; 16x2/20x2 only use the first two.)
const ROW_OFFSETS: [u8; 4] = [0x00, 0x40, 0x14, 0x54];

#[derive(Debug)]
pub enum LcdError {
    I2c(esp_hal::i2c::master::Error),
    Timeout,
}

impl From<esp_hal::i2c::master::Error> for LcdError {
    fn from(e: esp_hal::i2c::master::Error) -> Self {
        LcdError::I2c(e)
    }
}

impl From<TimeoutError> for LcdError {
    fn from(_: TimeoutError) -> Self {
        LcdError::Timeout
    }
}

pub struct Lcd {
    i2c: I2c<'static, esp_hal::Async>,
    address: u8,
    rgb_address: u8,
    width: u8,
    rows: u8,
}

impl Lcd {
    /// Create a new LCD instance
    pub fn new(i2c: I2c<'static, esp_hal::Async>, address: u8, rgb_address: u8) -> Self {
        Self {
            i2c,
            address,
            rgb_address,
            width: 16,
            rows: 2,
        }
    }

    pub fn rows(mut self, rows: u8) -> Self {
        // clamp so a bad config value can never index out of ROW_OFFSETS
        self.rows = rows.clamp(1, ROW_OFFSETS.len() as u8);
        self
    }

    pub fn width(mut self, width: u8) -> Self {
        self.width = width.max(1);
        self
    }

    /// One place all I2C writes go through, so every single call to the bus
    /// is timeout-guarded. If the bus is stuck (device unplugged, SDA held
    /// low, whatever), this returns LcdError::Timeout instead of hanging.
    async fn i2c_write(&mut self, addr: u8, bytes: &[u8]) -> Result<(), LcdError> {
        with_timeout(I2C_TIMEOUT, self.i2c.write_async(addr, bytes)).await??;
        Ok(())
    }

    /// Initialize the LCD and RGB backlight
    pub async fn init(&mut self) -> Result<(), LcdError> {
        // Initialize RGB backlight
        self.i2c_write(self.rgb_address, &[REG_MODE1, 0x00]).await?;
        self.i2c_write(self.rgb_address, &[REG_OUTPUT, 0xAA])
            .await?;
        self.i2c_write(self.rgb_address, &[REG_MODE2, 0x20]).await?;

        // Set backlight to white
        self.set_rgb(0, 125, 255).await?;
        Timer::after(Duration::from_millis(50)).await;

        // Initialize LCD
        self.send_command(LCD_FUNCTIONSET | LCD_2LINE | LCD_5X8DOTS)
            .await?;
        Timer::after(Duration::from_millis(5)).await;
        self.send_command(LCD_DISPLAYCONTROL | LCD_DISPLAYON | LCD_CURSOROFF | LCD_BLINKOFF)
            .await?;
        Timer::after(Duration::from_millis(5)).await;
        self.send_command(LCD_CLEARDISPLAY).await?;
        Timer::after(Duration::from_millis(2)).await;
        self.send_command(LCD_ENTRYMODESET | LCD_ENTRYLEFT | LCD_ENTRYSHIFTDECREMENT)
            .await?;
        Timer::after(Duration::from_millis(5)).await;
        Ok(())
    }

    /// Clear the display
    pub async fn clear(&mut self) -> Result<(), LcdError> {
        self.send_command(LCD_CLEARDISPLAY).await?;
        self.home().await?;
        Timer::after(Duration::from_millis(2)).await;
        Ok(())
    }

    /// Set the cursor position (row: 0..rows-1, col: 0..width-1)
    pub async fn set_cursor(&mut self, row: u8, col: u8) -> Result<(), LcdError> {
        // clamp instead of trusting caller input - no more out-of-bounds panic
        let row = row.min(self.rows.saturating_sub(1)) as usize;
        let col = col.min(self.width.saturating_sub(1));
        let pos = col + ROW_OFFSETS[row];
        self.send_command(LCD_SETDDRAMADDR | pos).await
    }

    /// Set the RGB backlight color (r, g, b: 0-255)
    pub async fn set_rgb(&mut self, r: u8, g: u8, b: u8) -> Result<(), LcdError> {
        self.i2c_write(self.rgb_address, &[REG_RED, r]).await?;
        self.i2c_write(self.rgb_address, &[REG_GREEN, g]).await?;
        self.i2c_write(self.rgb_address, &[REG_BLUE, b]).await?;
        Ok(())
    }

    /// Return cursor to home position (0,0)
    pub async fn home(&mut self) -> Result<(), LcdError> {
        self.send_command(LCD_RETURNHOME).await?;
        Timer::after(Duration::from_millis(2)).await;
        Ok(())
    }

    /// Send a command to the LCD
    async fn send_command(&mut self, cmd: u8) -> Result<(), LcdError> {
        self.i2c_write(self.address, &[0x80, cmd]).await
    }

    fn get_str_position(&self, s: &str, align: &TextAlign) -> u8 {
        // saturating so a string longer than `width` can't underflow into
        // garbage that gets used as a DDRAM address
        let len = (s.chars().count() as u8).min(self.width);
        match align {
            TextAlign::Left => 0,
            TextAlign::Center => (self.width.saturating_sub(len)) / 2,
            TextAlign::Right => self.width.saturating_sub(len),
        }
    }

    /// Write data to the LCD
    async fn write_data(&mut self, data: u8) -> Result<(), LcdError> {
        self.i2c_write(self.address, &[0x40, data]).await
    }

    pub async fn write_char(&mut self, c: char) -> Result<(), LcdError> {
        self.write_data(c as u8).await?;
        Timer::after(Duration::from_millis(1)).await;
        Ok(())
    }

    pub async fn write_str(
        &mut self,
        row: u8,
        s: &str,
        options: &WriteSettings,
    ) -> Result<(), LcdError> {
        if options.clear {
            self.clear().await?;
            self.home().await?;
        }
        let position = self.get_str_position(s, &options.align);
        self.set_cursor(row, position).await?;

        for c in s.chars().take(self.width as usize) {
            self.write_char(c).await?;
            if let Some(delay) = options.delay_per_char {
                Timer::after(Duration::from_millis(delay as u64)).await;
            }
        }
        Ok(())
    }

    pub async fn clear_row(&mut self, row: u8) -> Result<(), LcdError> {
        self.set_cursor(row, 0).await?;
        for _ in 0..self.width {
            self.write_char(' ').await?;
        }
        self.set_cursor(row, 0).await?;
        Ok(())
    }

    pub async fn write_str_no(&mut self, s: &str) -> Result<(), LcdError> {
        self.write_str(0, s, &WriteSettings::new()).await
    }
}

pub struct WriteSettings {
    align: TextAlign,
    delay_per_char: Option<u32>,
    clear: bool,
}

impl WriteSettings {
    pub fn new() -> Self {
        Self {
            align: TextAlign::Center,
            delay_per_char: None,
            clear: true,
        }
    }

    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    pub fn clear(mut self, clear: bool) -> Self {
        self.clear = clear;
        self
    }

    pub fn delay_per_char(mut self, delay: u32) -> Self {
        self.delay_per_char = Some(delay);
        self
    }
}

pub enum TextAlign {
    Left,
    Right,
    Center,
}
