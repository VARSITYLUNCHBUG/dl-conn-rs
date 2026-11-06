//! The text behind the F1 window and `show guide`. Only states what's been verified, and says so
//! where something hasn't been.

const GUIDE: &str = r"dl-rs help

Test your connection in a custom game before you trust it in a real match.
Work through the steps in order.


WHAT DL-RS DOES

dl-rs blocks every Steam relay except the server you pick, until you pick again
or restart the PC. It chooses which relays the game can reach. It does not make
any route faster, and no setting lowers ping beyond choosing a route.

A strict pick also removes the backup relay the game normally keeps. To keep a
failover route, set a default backup in F2.


STEP 1: TURN ON LOGGING

The report in F3 reads the game's own log, so the game has to write one.

  1. In Steam, open your Library.
  2. Right-click Deadlock and choose Properties.
  3. On the General tab, find Launch Options.
  4. Add this text:

         -condebug -lognetapi

  5. Close the window.

The log is written here, inside your Steam library:

  steamapps\common\Deadlock\game\citadel\console.log

The game rewrites this file every time it starts. Copy it first if you want to
keep one. To read a different game's log, set its path in F2.


STEP 2: PICK YOUR SERVER FIRST

  1. Close the game if it's running.
  2. Choose your server in the dl-rs dropdown. Type to search by city, code or
     region.
  3. Start the game.

Pick before the game starts, so the ping measurement it runs at startup already
sees the block. A block can't move a game that's already connected to a relay
you left open.


STEP 3: TEST IN A CUSTOM GAME

A custom game puts no match at risk, so use it to prove the setup first.

  1. Create a private lobby and start the game.
  2. Wait until you're in the match, then stay connected for at least 2 minutes.
  3. Leave the match.

The hideout doesn't count. It runs on your own PC, so it uses no relay and tells
you nothing about the route.


STEP 4: READ THE REPORT

Press F3. Findings are near the top, each marked with one of these:

  OK     This was checked and looks right.
  CHECK  This doesn't match what you asked for. Read the advice with it.
  NOTE   Something worth knowing. Nothing is wrong.

A working strict pick looks like this:

  Relays: 1 valid    The game could reach only the site you picked.
  Primary            On the site you picked.
  Backups            None. That's expected with no backup set.

What each CHECK usually means:

  More relays valid than sites open
      The block wasn't on when the game measured. Pick first, then restart the
      game. Close any other server picker, since its rules still apply.

  Primary isn't your pick
      The game connected somewhere else. Check that the log is from after your
      pick.

  Backup outside your open sites
      The game selected a relay that should have been blocked. If you cleared
      the block during that match, this is expected. If not, repeat the test
      with the block held and note the result.

  No relay valid
      The game could reach no relay. Check that your pick has relays online, and
      that no other tool is blocking them.

If the report says there's no SDR connection, that test didn't use a relay and
proves nothing about the route. One private lobby tested so far did connect
through a relay, but this hasn't been checked more widely.


STEP 5: MOVE TO A REAL MATCH

When the custom game passes, play a real match and press F3 again afterwards.
The report covers each connection in the log.


STEP 6: TURN LOGGING OFF

Remove -condebug and -lognetapi from Launch Options when you're finished. The
log grew by about 434 KB in 5 minutes in testing. You don't need these options
for blocking to work, only for the report and the route counter.


WHAT THE TWO COUNTERS MEASURE

Direct
    A ping from this PC straight to your pick. A game's real path can differ, so
    don't read it as your in-game ping.

Game
    The figure the game itself wrote to its log, with the time it was written.
    It's the last known value and it isn't live, because the game only logs it
    when it measures or connects. It needs the launch options from step 1.


IF YOU USE EXITLAG

You don't need ExitLag to use dl-rs. If you do use it:

  1. Choose the same server in ExitLag as in dl-rs. A block can't change where
     ExitLag routes.
  2. The second counter is labelled ExitLag route while ExitLag is running. A
     running ExitLag doesn't prove that the game was boosted at that moment.
  3. Direct doesn't go through ExitLag, so the two can differ a lot.
  4. If the report says the route looks slower than direct, try another route or
     server in ExitLag.


KEYS

  F1    This help
  F2    Settings
  F3    Log report
  `     Console
  Esc   Back to Status, or cancel a dropdown search
";

pub fn lines() -> Vec<String> {
    GUIDE.lines().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guide_is_plain_ascii_and_fits_the_window() {
        for line in lines() {
            assert!(line.is_ascii(), "{line}");
            assert!(line.len() <= 80, "too long: {line}");
        }
    }

    #[test]
    fn the_guide_covers_the_steps_in_order() {
        let text = lines().join("\n");
        let at = |part: &str| text.find(part).unwrap_or_else(|| panic!("missing {part}"));
        assert!(at("TURN ON LOGGING") < at("PICK YOUR SERVER"));
        assert!(at("PICK YOUR SERVER") < at("CUSTOM GAME\n"));
        assert!(at("CUSTOM GAME\n") < at("READ THE REPORT"));
        assert!(at("READ THE REPORT") < at("REAL MATCH"));
        assert!(at("REAL MATCH") < at("TURN LOGGING OFF"));
        assert!(text.contains("-condebug -lognetapi"));
    }

    #[test]
    fn exitlag_is_only_in_its_own_section() {
        let text = lines().join("\n");
        let section = text.find("IF YOU USE EXITLAG").unwrap();
        assert!(!text[..section].contains("ExitLag"));
        let keys = text.find("KEYS").unwrap();
        assert!(!text[keys..].contains("ExitLag"));
    }
}
