# esp32-drawer

This is meant to be a small learning project/exercise for myself to better
understand the capabilities of an ESP32. The goal of this project is to
serve a small Vue application from an ESP32 that allows the user to draw things
to a screen connected to the ESP32.

#### Notes/Considerations

* No Vue build step
    * The ESP32 serves an html file that links to Vue from a CDN

* Strip symbols
    * Strip is enabled on the release profile to reduce binary size

## Usage

This project makes use of an ESP32 and an ST7735 driven screen. The ESP32 is
**not** configured as an access point so a network connection is also required
and the web application is to be accessed through the network.

## Steps
1. Connect screen to ESP32
2. Define network ssid and password enviroment variables
3. Build binary and flash it to ESP32
4. Visit the local IP of the ESP32 in browser to access application

## Wiring

todo

## Environment Variables

Make sure these environment variables are set before building.

```shell
SSID=
PASSWORD=
```


