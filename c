## This is the darkirc configuration file.
## Review it carefully.

## JSON-RPC listen URL
#rpc_listen = "tcp://127.0.0.1:26660"

## IRC listen URL
#irc_listen = "tcp://127.0.0.1:6667"

## TLS certificate path if IRC acceptor uses TLS (optional)
#irc_tls_cert = "/etc/letsencrypt/darkirc/fullchain.pem"

## TLS secret key path if IRC acceptor uses TLS (optional)
#irc_tls_secret = "/etc/letsencrypt/darkirc/privkey.pem"

## Sets Datastore Path
#datastore = "~/.local/darkfi/darkirc/darkirc_db"

## Sets DB logs replay datastore path
#replay_datastore = "~/.local/darkfi/darkirc/replayed_darkirc_db"

## Run in replay mode to store Sled DB instructions
## (for eventgraph debugging tool)
#replay_mode = false

## List of channels to autojoin for new client connections
autojoin = [
    "#dev",
    "#memes",
    "#philosophy",
    "#markets",
    "#math",
    "#random",
    "#lunardao",
]

## IRC server specific password
## (optional, but once configured, it is required from the IRC client side)
#password = "CHANGE_ME"

## Number of attempts to sync the DAG.
#sync_attempts = 5

## Number of seconds to wait before trying again if sync fails.
#sync_timeout = 10

# Log to file. Off by default.
#log = "/tmp/darkirc.log"
# Set log level. 1 is info (default), 2 is debug, 3 is trace
#verbose = 2

# P2P network settings
[net]
# Path to the P2P datastore
datastore = "~/.local/darkfi/darkirc"

# Path to a configured hostlist for saving known peers
hostlist = "~/.local/darkfi/darkirc/p2p_hostlist.tsv"

## P2P accept addresses
#inbound = ["tcp+tls://0.0.0.0:26661", "tcp+tls://[::]:26661"]
#inbound = ["tor://127.0.0.1:26661"]

## Outbound connection slots
# outbound_connections = 8

## Inbound connection slots
#inbound_connections = 8

## White connection percent
# gold_connect_count = 2

## White connection percent
# white_connect_percent = 70

## Addresses we want to advertise to peers (optional)
## These should be reachable externally
#external_addrs = ["tcp+tls://my.resolveable.address:26661"]

## Seed nodes to connect to 
seeds = [
    #"tcp+tls://lilith0.dark.fi:5262",
    "tcp+tls://lilith1.dark.fi:5262",
    #"tor://czzulj66rr5kq3uhidzn7fh4qvt3vaxaoldukuxnl5vipayuj7obo7id.onion:5263",
    #"tor://vgbfkcu5hcnlnwd2lz26nfoa6g6quciyxwbftm6ivvrx74yvv5jnaoid.onion:5273",
]

## Manual peers to connect to
#peers = []

# Whitelisted transports for outbound connections
allowed_transports = ["tcp+tls"]
#allowed_transports = ["tor"]
#allowed_transports = ["tor", "tor+tls"]

# Enable transport mixing
# Allows mixing transports, e.g. tor+tls:// connecting to tcp+tls://
# By default this is not allowed.
transport_mixing = false

# Nodes to avoid interacting with for the duration of the program, in the
# format ["host", ["scheme", "scheme"], [port, port]].
# If scheme is left empty it will default to "tcp+tls". 
# If ports are left empty all ports from this peer will be blocked.
#blacklist = [["example.com", ["tcp"], [8551, 23331]]]

## ====================
## IRC channel settings
## ====================
##
## You can create a shared secret with `darkirc --gen-secret`.
## Never share this secret over unencrypted channels or with someone
## who you do not want to be able to read all the channel messages.
## Use it like this example:
#[channel."#foo"]
#secret = "7CkVuFgwTUpJn5Sv67Q3fyEDpa28yrSeL5Hg2GqQ4jfM"
#topic = "My secret channel"

[channel."#dev"]
topic = "DarkFi Development HQ"

[channel."#markets"]
topic = "Crypto Market Talk"

[channel."#math"]
topic = "Math Talk"

[channel."#memes"]
topic = "DarkFi Meme Reality"

[channel."#philosophy"]
topic = "Philosophy Discussions"

[channel."#random"]
topic = "/b/"

[channel."#lunardao"]
topic = "LunarDAO talk"

## ================
## Contact settings
## ================
##
## In this section we configure our contacts and people we want to
## have encrypted DMs with. Your contacts' public keys should be
## retrieved manually. Whenever this is changed, you can send a
## SIGHUP signal to the running darkirc instance to reload these.
##
## The secret key used to decrypt direct messages sent to your public
## key (the counterpart to this secret key).
## It is also recommended to paste the public key here as a comment in
## order to be able to easily reference it for sharing.
##
## You can generate a keypair with: darkirc --gen-chacha-keypair
## and replace the secret key below with the generated one.
## **You should never share this secret key with anyone**
#[crypto]
#dm_chacha_secret = "AKfyoKxnHb8smqP2zt9BVvXkcN7pm9GnqqyuYRmxmWtR"

## This is where you put other people's public keys. The format is:
## [contact."nickname"]. "nickname" can be anything you want.
## This is how they will appear in your IRC client when they send you a DM.
##
## Example (set as many as you want):
#[contact."satoshi"]
#dm_chacha_public = "C9vC6HNDfGQofWCapZfQK5MkV1JR8Cct839RDUCqbDGK"
#
#[contact."anon"]
#dm_chacha_public = "7iTddcopP2pkvszFjbFUr7MwTcMSKZkYP6zUan22pxfX"
