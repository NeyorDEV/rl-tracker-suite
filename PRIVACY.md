# Privacy Policy for RL Tracker Suite

**RL Tracker Suite** respects your privacy. This policy describes how data is handled by the application:

1. **No Personal Data Collection**: RL Tracker Suite does not collect, record, track, or share any personally identifiable information (PII).
2. **Local Processing**: All match statistics, session tracking, and game telemetry are received directly from Rocket League's local TCP port (`127.0.0.1:49123`) and are processed solely in memory and in local configuration files on your own machine.
3. **No Third-Party Telemetry**: There are no analytic tracking SDKs, no advertising trackers, and no remote crash reporters embedded in the software.
4. **Third-Party Lookups**: When using the optional MMR sync feature, public rank data is fetched directly from public stats providers (such as Rocket League Tracker Network) using the player identifier you specify. No session data or system information is transmitted.
5. **Contact**: For any questions regarding privacy or source code integrity, please open an issue on the official [GitHub repository](https://github.com/NeyorDEV/rl-session-stats).

## LobbyTracker tab

Players of your current lobby (their public platform name or ID, taken from the local Stats API) are looked up on Tracker Network from a hidden browser window, like your own MMR sync. Results are cached locally in `lobby_cache.db`. Nothing is sent anywhere else, and nothing is read from or written to the game process.
