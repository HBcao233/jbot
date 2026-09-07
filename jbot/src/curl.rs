use wreq_util::Emulation::Chrome137;

pub fn get_client() -> wreq::Result<wreq::Client> {
    wreq::Client::builder().emulation(Chrome137).build()
}
