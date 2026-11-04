### Checking your download (optional)

`SHA256SUMS.txt` lists a code (a kind of fingerprint) for each program. If the program you downloaded gives the same code, it's exactly the file that was built. Unzip your download first: the codes are for the program inside the zip, not for the zip.

**Windows**
1. Open **PowerShell** from the Start menu.
2. Type `Get-FileHash ` (with a space at the end), drag `litanatomy.exe` into the window and press Enter.
3. Double-click the long code under **Hash** to select it, and copy it with Ctrl+C.
4. Open `SHA256SUMS.txt` in Notepad, press Ctrl+F and paste. It should be found on the line ending in `litanatomy-Windows/litanatomy.exe`.

**macOS**
1. Open **Terminal** (Applications › Utilities).
2. Type `shasum -a 256 ` (with a space at the end), drag the unzipped `litanatomy` file into the window and press Enter.
3. Double-click the long code to select it, and copy it with Cmd+C.
4. Open `SHA256SUMS.txt` in TextEdit, press Cmd+F and paste. It should be found on the line ending in `litanatomy-macOS/litanatomy`.

**Linux**
In the folder with the zip and `SHA256SUMS.txt`, run:

    unzip litanatomy-Linux.zip -d litanatomy-Linux
    sha256sum -c --ignore-missing SHA256SUMS.txt

It should print `litanatomy-Linux/litanatomy: OK`.

If the code isn't found, or Linux says `FAILED`, don't run the program. Download it again, and get in touch if it still doesn't match.
