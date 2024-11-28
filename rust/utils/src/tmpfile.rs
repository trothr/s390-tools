// SPDX-License-Identifier: MIT
//
// Copyright IBM Corp. 2024

use std::{
    ffi::{CString, OsStr},
    os::unix::prelude::OsStrExt,
    path::{Path, PathBuf},
};

/// Rust wrapper for `libc::mkdtemp`
fn mkdtemp<P: AsRef<Path>>(template: P) -> Result<PathBuf, std::io::Error> {
    let template_cstr = CString::new(template.as_ref().as_os_str().as_bytes())?;
    let template_raw = template_cstr.into_raw();
    unsafe {
        // SAFETY: template_raw is a valid CString because it was generated by
        // the `CString::new`.
        let ret = libc::mkdtemp(template_raw);

        if ret.is_null() {
            Err(std::io::Error::last_os_error())
        } else {
            // SAFETY: `template_raw` is still a valid CString because it was
            // generated by `CString::new` and modified by `libc::mkdtemp`.
            let path_cstr = std::ffi::CString::from_raw(template_raw);
            let path = OsStr::from_bytes(path_cstr.as_bytes());
            let path = std::path::PathBuf::from(path);

            Ok(path)
        }
    }
}

/// This type creates a temporary directory that is automatically removed when
/// it goes out of scope. It utilizes the `mkdtemp` function and its semantics,
/// with the addition of automatically including the template characters
/// `XXXXXX`.
#[derive(PartialEq, Eq, Debug)]
pub struct TemporaryDirectory {
    path: Box<Path>,
}

impl TemporaryDirectory {
    /// Creates a temporary directory in the current working directory using
    /// 'tmp.' as directory prefix.
    ///
    /// # Errors
    ///
    /// This function will return an error if the temporary directory could not
    /// be created.
    ///
    /// # Example
    ///
    /// ```
    /// # use utils::TemporaryDirectory;
    /// let temp = TemporaryDirectory::new().unwrap();
    /// ```
    pub fn new() -> Result<Self, std::io::Error> {
        Self::with_prefix("tmp.")
    }

    /// Creates a temporary directory in the current working directory using
    /// `prefix` as directory prefix.
    ///
    /// # Errors
    ///
    /// This function will return an error if the temporary directory could not
    /// created.
    ///
    /// # Example
    ///
    /// ```
    /// # use utils::TemporaryDirectory;
    /// let temp = TemporaryDirectory::with_prefix("test").unwrap();
    /// ```
    pub fn with_prefix<P: AsRef<Path>>(prefix: P) -> Result<Self, std::io::Error> {
        let mut template = prefix.as_ref().to_owned();
        let template_os_string = template.as_mut_os_string();
        template_os_string.push("XXXXXX");

        let temp_dir = mkdtemp(template_os_string)?;
        Ok(Self {
            path: temp_dir.into_boxed_path(),
        })
    }

    /// Returns a reference to the path of the created temporary directory.
    pub fn path(&self) -> &Path {
        self.path.as_ref()
    }

    /// Takes ownership and releases the memory and makes sure no destructor is
    /// called and therefore the temporary directory will not be removed.
    fn forget(mut self) {
        self.path = PathBuf::new().into_boxed_path();
        std::mem::forget(self);
    }

    /// Removes the created temporary directory and it's contents.
    ///
    /// # Errors
    ///
    /// This function will return an error if the temporary directory could not
    /// removed.
    pub fn close(self) -> std::io::Result<()> {
        let ret = std::fs::remove_dir_all(&self.path);
        self.forget();
        ret
    }
}

impl AsRef<Path> for TemporaryDirectory {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::{mkdtemp, TemporaryDirectory};

    #[test]
    fn mkdtemp_test() {
        let template_inv_not_last_characters = "XXXXXXyay";
        let template_inv_too_less_x = "yayXXXXX";
        let template_inv_path_does_not_exist = "../NA-yay/XXXXXX";

        let template = "yayXXXXXX";

        let _err = mkdtemp(template_inv_not_last_characters).expect_err("invalid template");
        let _err = mkdtemp(template_inv_too_less_x).expect_err("invalid template");
        let _err =
            mkdtemp(template_inv_path_does_not_exist).expect_err("path does not exist template");

        let path = mkdtemp(template).expect("mkdtemp should work");
        assert!(path.exists());
        assert!(path.as_os_str().to_str().expect("works").starts_with("yay"));
        std::fs::remove_dir(path).unwrap();
    }

    #[test]
    fn temporary_directory_resides_in_cwd() {
        let temp_dir = TemporaryDirectory::new().expect("should work");
        let path = temp_dir.path().to_owned();
        let cwd = std::env::current_dir().unwrap();

        assert_eq!(path.canonicalize().unwrap().parent().unwrap(), cwd);
    }

    #[test]
    fn temporary_directory_close_test() {
        let temp_dir = TemporaryDirectory::new().expect("should work");
        let path = temp_dir.path().to_owned();
        assert!(path.exists());

        // Test that close removes the directory
        temp_dir.close().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn temporary_directory_drop_test() {
        let temp_dir = TemporaryDirectory::new().expect("should work");
        let path = temp_dir.path().to_owned();
        assert!(path.exists());

        // Test that the destructor removes the directory
        drop(temp_dir);
        assert!(!path.exists());
    }

    #[test]
    fn temporary_directory_prefix_test() {
        let prefix = "yay";
        let temp_dir = TemporaryDirectory::with_prefix(prefix).expect("should work");

        let path = temp_dir.path().to_owned();
        assert!(path.exists());
        assert!(path
            .as_os_str()
            .to_str()
            .expect("works")
            .starts_with(prefix));
    }

    #[test]
    fn temporary_directory_empty_prefix_test() {
        let temp_dir = TemporaryDirectory::with_prefix("").expect("should work");
        let path = temp_dir.path().to_owned();
        assert!(path.exists());
        // Path consists only of the rendered template.
        assert_eq!(path.as_os_str().len(), "XXXXXX".len());
    }

    #[test]
    fn temporary_directory_as_ref_test() {
        let temp_dir = TemporaryDirectory::new().expect("should work");

        assert_eq!(temp_dir.path(), temp_dir.as_ref());
    }
}
